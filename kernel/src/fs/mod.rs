use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use spin::{Lazy, Mutex};

use crate::auth::permissions::protected_path_owner;
use crate::auth::session;
use crate::auth::{FilePermissions, UserRole};
use crate::arch::x86_64::interrupts;
use crate::memory;
use crate::{boot_complete_ms, BUILD_DATE, KERNEL_VERSION};

pub const MAX_FILES: usize = 128;
pub const MAX_FILE_SIZE: usize = 64 * 1024;
pub const TOTAL_CAPACITY: usize = 4 * 1024 * 1024;
pub const MAX_PATH_LEN: usize = 96;
const MAX_COMPONENT_LEN: usize = 31;

pub static FILESYSTEM: Lazy<Mutex<WarFS>> = Lazy::new(|| Mutex::new(WarFS::new()));

pub struct WarFS {
    files: Vec<FileEntry>,
    max_files: usize,
}

#[derive(Clone)]
pub struct FileEntry {
    pub name: String,
    pub data: Vec<u8>,
    pub created_at: u64,
    pub modified_at: u64,
    pub readonly: bool,
    pub owner_uid: u16,
    pub permissions: FilePermissions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    FileNotFound,
    FilesystemFull,
    FilenameTooLong,
    FileTooLarge,
    ReadOnly,
    InvalidFilename,
    PermissionDenied,
}

impl WarFS {
    #[must_use]
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            max_files: MAX_FILES,
        }
    }

    pub fn write(&mut self, name: &str, data: &[u8]) -> Result<(), FsError> {
        self.write_system(name, data, false)
    }

    pub fn write_readonly(&mut self, name: &str, data: &[u8]) -> Result<(), FsError> {
        self.write_system(name, data, true)
    }

    pub fn write_system(
        &mut self,
        name: &str,
        data: &[u8],
        readonly: bool,
    ) -> Result<(), FsError> {
        self.write_internal(name, data, 0, UserRole::Admin, FilePermissions::system(), readonly)
    }

    pub fn write_as(
        &mut self,
        name: &str,
        data: &[u8],
        current_uid: u16,
        current_role: UserRole,
        permissions: FilePermissions,
    ) -> Result<(), FsError> {
        self.write_internal(name, data, current_uid, current_role, permissions, false)
    }

    pub fn touch(&mut self, name: &str) -> Result<(), FsError> {
        self.write(name, &[])
    }

    pub fn touch_as(
        &mut self,
        name: &str,
        current_uid: u16,
        current_role: UserRole,
        permissions: FilePermissions,
    ) -> Result<(), FsError> {
        self.write_as(name, &[], current_uid, current_role, permissions)
    }

    pub fn read(&self, name: &str) -> Result<&[u8], FsError> {
        self.read_as(name, 0, UserRole::Admin)
    }

    pub fn read_as(
        &self,
        name: &str,
        current_uid: u16,
        current_role: UserRole,
    ) -> Result<&[u8], FsError> {
        let canonical = canonical_path(name)?;
        let entry = self.find(&canonical).ok_or(FsError::FileNotFound)?;
        if !entry.permissions.can_read(current_uid, current_role) {
            return Err(FsError::PermissionDenied);
        }
        Ok(&entry.data)
    }

    pub fn delete(&mut self, name: &str) -> Result<(), FsError> {
        self.delete_as(name, 0, UserRole::Admin)
    }

    pub fn delete_as(
        &mut self,
        name: &str,
        current_uid: u16,
        current_role: UserRole,
    ) -> Result<(), FsError> {
        let canonical = canonical_path(name)?;
        let index = self
            .files
            .iter()
            .position(|entry| entry.name == canonical)
            .ok_or(FsError::FileNotFound)?;
        let entry = &self.files[index];
        if entry.readonly {
            return Err(FsError::ReadOnly);
        }
        if current_role != UserRole::Admin && entry.owner_uid != current_uid {
            return Err(FsError::PermissionDenied);
        }
        self.files.remove(index);
        crate::disk::maybe_delete_path(&canonical);
        Ok(())
    }

    #[must_use]
    pub fn list(&self) -> &[FileEntry] {
        &self.files
    }

    pub fn list_dir_as(
        &self,
        directory: &str,
        current_uid: u16,
        current_role: UserRole,
    ) -> Result<Vec<FileEntry>, FsError> {
        let canonical = canonical_directory(directory)?;
        let mut entries = Vec::new();
        for entry in &self.files {
            if parent_directory(&entry.name) == canonical
                && entry.permissions.can_read(current_uid, current_role)
            {
                entries.push(entry.clone());
            }
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    #[must_use]
    pub fn exists(&self, name: &str) -> bool {
        canonical_path(name)
            .ok()
            .is_some_and(|canonical| self.find(&canonical).is_some())
    }

    #[must_use]
    pub fn free_space(&self) -> usize {
        TOTAL_CAPACITY.saturating_sub(self.used_space())
    }

    #[must_use]
    pub fn used_space(&self) -> usize {
        self.files.iter().map(|entry| entry.data.len()).sum()
    }

    pub fn stat(&self, name: &str) -> Result<&FileEntry, FsError> {
        self.stat_as(name, 0, UserRole::Admin)
    }

    pub fn stat_as(
        &self,
        name: &str,
        current_uid: u16,
        current_role: UserRole,
    ) -> Result<&FileEntry, FsError> {
        let canonical = canonical_path(name)?;
        let entry = self.find(&canonical).ok_or(FsError::FileNotFound)?;
        if !entry.permissions.can_read(current_uid, current_role) {
            return Err(FsError::PermissionDenied);
        }
        Ok(entry)
    }

    pub fn chmod_as(
        &mut self,
        name: &str,
        current_uid: u16,
        current_role: UserRole,
        mode: &str,
    ) -> Result<(), FsError> {
        let canonical = canonical_path(name)?;
        let entry = self.find_mut(&canonical).ok_or(FsError::FileNotFound)?;
        if entry.readonly {
            return Err(FsError::ReadOnly);
        }
        if current_role != UserRole::Admin && entry.owner_uid != current_uid {
            return Err(FsError::PermissionDenied);
        }
        if !entry.permissions.apply_mode_string(mode) {
            return Err(FsError::InvalidFilename);
        }
        if let Some(entry) = self.find(&canonical) {
            crate::disk::maybe_sync_file_entry(entry);
        }
        Ok(())
    }

    pub fn load_file_from_disk(
        &mut self,
        name: &str,
        data: &[u8],
        owner_uid: u16,
        mut permissions: FilePermissions,
        readonly: bool,
        created_at: u64,
        modified_at: u64,
    ) -> Result<(), FsError> {
        if data.len() > MAX_FILE_SIZE {
            return Err(FsError::FileTooLarge);
        }

        let canonical = canonical_path(name)?;
        if canonical == "/" {
            return Err(FsError::InvalidFilename);
        }

        let used_without_target = self
            .files
            .iter()
            .filter(|entry| entry.name != canonical)
            .map(|entry| entry.data.len())
            .sum::<usize>();
        if used_without_target.saturating_add(data.len()) > TOTAL_CAPACITY {
            return Err(FsError::FilesystemFull);
        }

        permissions.owner_uid = owner_uid;
        if let Some(entry) = self.find_mut(&canonical) {
            entry.data.clear();
            entry.data.extend_from_slice(data);
            entry.created_at = created_at;
            entry.modified_at = modified_at;
            entry.readonly = readonly;
            entry.owner_uid = owner_uid;
            entry.permissions = permissions;
            return Ok(());
        }

        if self.files.len() >= self.max_files {
            return Err(FsError::FilesystemFull);
        }

        self.files.push(FileEntry {
            name: canonical,
            data: data.to_vec(),
            created_at,
            modified_at,
            readonly,
            owner_uid,
            permissions,
        });
        self.files.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(())
    }

    #[must_use]
    pub fn owned_file_count(&self, uid: u16) -> usize {
        self.files.iter().filter(|entry| entry.owner_uid == uid).count()
    }

    fn write_internal(
        &mut self,
        name: &str,
        data: &[u8],
        current_uid: u16,
        current_role: UserRole,
        permissions: FilePermissions,
        readonly: bool,
    ) -> Result<(), FsError> {
        if data.len() > MAX_FILE_SIZE {
            return Err(FsError::FileTooLarge);
        }

        let canonical = canonical_path(name)?;
        if canonical == "/" {
            return Err(FsError::InvalidFilename);
        }

        let now = interrupts::tick_count();
        let used_without_target = self
            .files
            .iter()
            .filter(|entry| entry.name != canonical)
            .map(|entry| entry.data.len())
            .sum::<usize>();
        if used_without_target.saturating_add(data.len()) > TOTAL_CAPACITY {
            return Err(FsError::FilesystemFull);
        }

        if let Some(entry) = self.find_mut(&canonical) {
            if entry.readonly {
                return Err(FsError::ReadOnly);
            }
            if !entry.permissions.can_write(current_uid, current_role) {
                return Err(FsError::PermissionDenied);
            }
            entry.data.clear();
            entry.data.extend_from_slice(data);
            entry.modified_at = now;
            entry.readonly = readonly;
        } else {
            if self.files.len() >= self.max_files {
                return Err(FsError::FilesystemFull);
            }

            self.files.push(FileEntry {
                name: canonical.clone(),
                data: data.to_vec(),
                created_at: now,
                modified_at: now,
                readonly,
                owner_uid: permissions.owner_uid,
                permissions,
            });
            self.files.sort_by(|left, right| left.name.cmp(&right.name));
        }

        if let Some(entry) = self.find(&canonical) {
            crate::disk::maybe_sync_file_entry(entry);
        }

        Ok(())
    }

    fn find(&self, canonical: &str) -> Option<&FileEntry> {
        self.files.iter().find(|entry| entry.name == canonical)
    }

    fn find_mut(&mut self, canonical: &str) -> Option<&mut FileEntry> {
        self.files.iter_mut().find(|entry| entry.name == canonical)
    }
}

impl Default for WarFS {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Display for FsError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FileNotFound => formatter.write_str("file not found"),
            Self::FilesystemFull => formatter.write_str("filesystem full"),
            Self::FilenameTooLong => formatter.write_str("filename too long"),
            Self::FileTooLarge => formatter.write_str("file exceeds 64 KiB limit"),
            Self::ReadOnly => formatter.write_str("file is read-only"),
            Self::InvalidFilename => formatter.write_str("invalid filename"),
            Self::PermissionDenied => formatter.write_str("permission denied"),
        }
    }
}

pub fn init() {
    drop(FILESYSTEM.lock());
}

pub fn seed_system_files() -> Result<(), FsError> {
    let persistence_line = if crate::disk::is_available() {
        "Persistent disk: active (virtio-blk backing store).\n"
    } else {
        "Persistent disk: unavailable, user files are lost on reboot.\n"
    };
    let readme = concat!(
        "WarOS v0.2.0 - Quantum-Classical Hybrid Operating System\n",
        "War Enterprise - Building the future of computing\n\n",
        "Use 'help fs' for filesystem commands.\n",
        "Use 'help quantum' for quantum computing commands.\n\n",
        "warenterprise.com/waros\n",
        "github.com/WarEnterprise/waros\n",
    );
    let readme = format!("{readme}{persistence_line}");

    let stats = memory::stats();
    let sysinfo = format!(
        "WarOS v{KERNEL_VERSION}\n\
Kernel: waros-kernel {KERNEL_VERSION}\n\
Built: {BUILD_DATE}\n\
Boot time: {} ms\n\
RAM: {} MiB ({} frames)\n\
Heap: {} MiB\n\
Quantum: StateVector (18 qubits max)\n\
Filesystem: {} files max, {} KiB max per file\n",
        boot_complete_ms(),
        (stats.total_frames * 4) / 1024,
        stats.total_frames,
        crate::memory::heap::HEAP_SIZE / (1024 * 1024),
        MAX_FILES,
        MAX_FILE_SIZE / 1024
    );

    let mut filesystem = FILESYSTEM.lock();
    filesystem.write_system("/readme.txt", readme.as_bytes(), true)?;
    filesystem.write_system("/sysinfo.txt", sysinfo.as_bytes(), true)?;
    Ok(())
}

pub fn write_current(name: &str, data: &[u8]) -> Result<String, FsError> {
    let resolved = session::resolve_path(name);
    if !session::can_access_path(&resolved, true) {
        return Err(FsError::PermissionDenied);
    }

    let uid = session::current_uid();
    let role = session::current_role();
    let permissions = default_permissions_for(&resolved, uid);
    FILESYSTEM
        .lock()
        .write_as(&resolved, data, uid, role, permissions)?;
    Ok(resolved)
}

pub fn touch_current(name: &str) -> Result<String, FsError> {
    let resolved = session::resolve_path(name);
    if !session::can_access_path(&resolved, true) {
        return Err(FsError::PermissionDenied);
    }

    let uid = session::current_uid();
    let role = session::current_role();
    let permissions = default_permissions_for(&resolved, uid);
    FILESYSTEM
        .lock()
        .touch_as(&resolved, uid, role, permissions)?;
    Ok(resolved)
}

pub fn read_current(name: &str) -> Result<(String, Vec<u8>), FsError> {
    let resolved = session::resolve_path(name);
    if !session::can_access_path(&resolved, false) {
        return Err(FsError::PermissionDenied);
    }

    let uid = session::current_uid();
    let role = session::current_role();
    let data = FILESYSTEM.lock().read_as(&resolved, uid, role)?.to_vec();
    Ok((resolved, data))
}

pub fn delete_current(name: &str) -> Result<String, FsError> {
    let resolved = session::resolve_path(name);
    if !session::can_access_path(&resolved, true) {
        return Err(FsError::PermissionDenied);
    }

    let uid = session::current_uid();
    let role = session::current_role();
    FILESYSTEM.lock().delete_as(&resolved, uid, role)?;
    Ok(resolved)
}

pub fn stat_current(name: &str) -> Result<FileEntry, FsError> {
    let resolved = session::resolve_path(name);
    if !session::can_access_path(&resolved, false) {
        return Err(FsError::PermissionDenied);
    }

    let uid = session::current_uid();
    let role = session::current_role();
    FILESYSTEM.lock().stat_as(&resolved, uid, role).cloned()
}

pub fn chmod_current(name: &str, mode: &str) -> Result<String, FsError> {
    let resolved = session::resolve_path(name);
    if !session::can_access_path(&resolved, true) {
        return Err(FsError::PermissionDenied);
    }

    let uid = session::current_uid();
    let role = session::current_role();
    FILESYSTEM.lock().chmod_as(&resolved, uid, role, mode)?;
    Ok(resolved)
}

pub fn list_current(path: Option<&str>) -> Result<(String, Vec<FileEntry>), FsError> {
    let directory = path
        .map(session::resolve_path)
        .unwrap_or_else(session::current_home);
    if !session::can_access_path(&directory, false) {
        return Err(FsError::PermissionDenied);
    }

    let uid = session::current_uid();
    let role = session::current_role();
    let entries = FILESYSTEM.lock().list_dir_as(&directory, uid, role)?;
    Ok((directory, entries))
}

#[must_use]
pub fn display_path(path: &str) -> String {
    let home = session::current_home();
    if home != "/" && path == home {
        return String::from("~");
    }
    if home != "/" && path.starts_with(&(home.clone() + "/")) {
        return alloc::format!("~{}", &path[home.len()..]);
    }
    path.to_string()
}

#[must_use]
pub fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[must_use]
pub fn owner_label(path: &str) -> Option<(u16, String)> {
    protected_path_owner(path)
}

#[must_use]
pub fn format_timestamp(ticks: u64) -> String {
    let seconds = ticks / 100;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[must_use]
pub fn file_count_for_user(uid: u16) -> usize {
    FILESYSTEM.lock().owned_file_count(uid)
}

fn default_permissions_for(path: &str, uid: u16) -> FilePermissions {
    if path == "/root" || path.starts_with("/root/") || path.starts_with("/home/") {
        FilePermissions::private(uid)
    } else {
        FilePermissions::shared(uid)
    }
}

fn canonical_directory(path: &str) -> Result<String, FsError> {
    let canonical = canonical_path(path)?;
    Ok(if canonical.is_empty() {
        String::from("/")
    } else {
        canonical
    })
}

fn canonical_path(path: &str) -> Result<String, FsError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(FsError::InvalidFilename);
    }

    let normalized = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };

    let mut result = String::from("/");
    let mut first = true;
    for component in normalized.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            return Err(FsError::InvalidFilename);
        }
        if component.len() > MAX_COMPONENT_LEN
            || !component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(FsError::InvalidFilename);
        }
        if !first {
            result.push('/');
        }
        result.push_str(component);
        first = false;
    }

    if result.len() > MAX_PATH_LEN {
        return Err(FsError::FilenameTooLong);
    }

    Ok(result)
}

fn parent_directory(path: &str) -> String {
    if path == "/" {
        return String::from("/");
    }

    let trimmed = path.trim_end_matches('/');
    if let Some(index) = trimmed.rfind('/') {
        if index == 0 {
            String::from("/")
        } else {
            trimmed[..index].to_string()
        }
    } else {
        String::from("/")
    }
}
