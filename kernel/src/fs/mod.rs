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

#[derive(Clone)]
pub struct DirEntryView {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: usize,
    pub modified_at: u64,
    pub readonly: bool,
    pub owner_uid: u16,
    pub permissions: FilePermissions,
}

#[derive(Clone)]
pub struct GrepMatch {
    pub line_number: usize,
    pub line: String,
}

#[derive(Clone, Copy)]
pub struct WordCount {
    pub lines: usize,
    pub words: usize,
    pub bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    FileNotFound,
    AlreadyExists,
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

    pub fn seed_system(
        &mut self,
        name: &str,
        data: &[u8],
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
            entry.data.clear();
            entry.data.extend_from_slice(data);
            entry.modified_at = now;
            entry.readonly = readonly;
            entry.owner_uid = 0;
            entry.permissions = FilePermissions::system();
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
                owner_uid: 0,
                permissions: FilePermissions::system(),
            });
            self.files.sort_by(|left, right| left.name.cmp(&right.name));
        }

        if let Some(entry) = self.find(&canonical) {
            crate::disk::maybe_sync_file_entry(entry);
        }

        Ok(())
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

    pub fn create_new_as(
        &mut self,
        name: &str,
        data: &[u8],
        owner_uid: u16,
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
        if self.find(&canonical).is_some() {
            return Err(FsError::AlreadyExists);
        }

        let parent = parent_directory(&canonical);
        if !self.directory_exists_canonical(&parent) {
            return Err(FsError::FileNotFound);
        }
        if self.files.len() >= self.max_files {
            return Err(FsError::FilesystemFull);
        }
        if self.used_space().saturating_add(data.len()) > TOTAL_CAPACITY {
            return Err(FsError::FilesystemFull);
        }

        let now = interrupts::tick_count();
        self.files.push(FileEntry {
            name: canonical.clone(),
            data: data.to_vec(),
            created_at: now,
            modified_at: now,
            readonly,
            owner_uid,
            permissions,
        });
        self.files.sort_by(|left, right| left.name.cmp(&right.name));

        if let Some(entry) = self.find(&canonical) {
            crate::disk::maybe_sync_file_entry(entry);
        }

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

    pub fn find(&self, canonical: &str) -> Option<&FileEntry> {
        self.files.iter().find(|entry| entry.name == canonical)
    }

    fn find_mut(&mut self, canonical: &str) -> Option<&mut FileEntry> {
        self.files.iter_mut().find(|entry| entry.name == canonical)
    }

    fn directory_exists_canonical(&self, path: &str) -> bool {
        if path == "/" {
            return true;
        }
        let marker_path = directory_marker_path(path);
        self.files.iter().any(|entry| {
            entry.name == marker_path || entry.name.starts_with(&(path.to_string() + "/"))
        })
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
            Self::AlreadyExists => formatter.write_str("file already exists"),
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

fn seed_system_path(
    filesystem: &mut WarFS,
    path: &str,
    data: &[u8],
    readonly: bool,
) -> Result<(), FsError> {
    filesystem.seed_system(path, data, readonly).map_err(|error| {
        crate::serial_println!("[ERR] WarFS seed {}: {}", path, error);
        error
    })
}

fn clear_seed_output(filesystem: &mut WarFS, path: &str) -> Result<(), FsError> {
    match filesystem.delete(path) {
        Ok(()) | Err(FsError::FileNotFound) => Ok(()),
        Err(error) => {
            crate::serial_println!("[ERR] WarFS reset {}: {}", path, error);
            Err(error)
        }
    }
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
    seed_system_path(&mut filesystem, "/readme.txt", readme.as_bytes(), true)?;
    seed_system_path(&mut filesystem, "/sysinfo.txt", sysinfo.as_bytes(), true)?;
    let smoke_elf = crate::exec::smoke::elf_bytes();
    seed_system_path(&mut filesystem, crate::exec::smoke::SMOKE_ELF_PATH, &smoke_elf, true)?;
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_READ_SMOKE_FILE_PATH,
        crate::exec::smoke::ABI_READ_SMOKE_FILE_CONTENT.as_bytes(),
        true,
    )?;
    let abi_read_elf = crate::exec::smoke::abi_read_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_READ_SMOKE_ELF_PATH,
        &abi_read_elf,
        true,
    )?;
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_OFFSET_SMOKE_FILE_PATH,
        crate::exec::smoke::ABI_OFFSET_SMOKE_FILE_CONTENT.as_bytes(),
        true,
    )?;
    let abi_offset_elf = crate::exec::smoke::abi_offset_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_OFFSET_SMOKE_ELF_PATH,
        &abi_offset_elf,
        true,
    )?;
    let abi_argv_elf = crate::exec::smoke::abi_argv_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_ARGV_SMOKE_ELF_PATH,
        &abi_argv_elf,
        true,
    )?;
    let abi_exec_parent_elf = crate::exec::smoke::abi_exec_parent_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_EXEC_PARENT_ELF_PATH,
        &abi_exec_parent_elf,
        true,
    )?;
    let abi_exec_child_elf = crate::exec::smoke::abi_exec_child_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_EXEC_CHILD_ELF_PATH,
        &abi_exec_child_elf,
        true,
    )?;
    let abi_heap_elf = crate::exec::smoke::abi_heap_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_HEAP_SMOKE_ELF_PATH,
        &abi_heap_elf,
        true,
    )?;
    let abi_fault_elf = crate::exec::smoke::abi_fault_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_FAULT_SMOKE_ELF_PATH,
        &abi_fault_elf,
        true,
    )?;
    let abi_wait_parent_elf = crate::exec::smoke::abi_wait_parent_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_WAIT_PARENT_ELF_PATH,
        &abi_wait_parent_elf,
        true,
    )?;
    let abi_wait_child_elf = crate::exec::smoke::abi_wait_child_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_WAIT_CHILD_ELF_PATH,
        &abi_wait_child_elf,
        true,
    )?;
    let abi_stat_elf = crate::exec::smoke::abi_stat_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_STAT_SMOKE_ELF_PATH,
        &abi_stat_elf,
        true,
    )?;
    seed_system_path(
        &mut filesystem,
        &directory_marker_path(crate::exec::smoke::ABI_READDIR_SMOKE_DIR_PATH),
        &[],
        true,
    )?;
    seed_system_path(
        &mut filesystem,
        "/abi/readdir-proof/alpha.txt",
        b"alpha dirent proof\n",
        true,
    )?;
    seed_system_path(
        &mut filesystem,
        "/abi/readdir-proof/beta.txt",
        b"beta dirent proof\n",
        true,
    )?;
    seed_system_path(
        &mut filesystem,
        "/abi/readdir-proof/gamma.txt",
        b"gamma dirent proof\n",
        true,
    )?;
    let abi_readdir_elf = crate::exec::smoke::abi_readdir_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_READDIR_SMOKE_ELF_PATH,
        &abi_readdir_elf,
        true,
    )?;
    let abi_path_elf = crate::exec::smoke::abi_path_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_PATH_SMOKE_ELF_PATH,
        &abi_path_elf,
        true,
    )?;
    seed_system_path(
        &mut filesystem,
        &directory_marker_path(crate::exec::smoke::ABI_WRITE_SMOKE_DIR_PATH),
        &[],
        true,
    )?;
    clear_seed_output(&mut filesystem, crate::exec::smoke::ABI_WRITE_SMOKE_FILE_PATH)?;
    let abi_write_elf = crate::exec::smoke::abi_write_elf_bytes();
    seed_system_path(
        &mut filesystem,
        crate::exec::smoke::ABI_WRITE_SMOKE_ELF_PATH,
        &abi_write_elf,
        true,
    )?;
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
    let mut fs = FILESYSTEM.lock();
    let existed = fs.find(&resolved).is_some();
    fs.write_as(&resolved, data, uid, role, permissions)?;
    drop(fs);

    crate::security::audit::log_event(if existed {
        crate::security::audit::events::AuditEvent::FileModified {
            path: resolved.clone(),
            uid,
        }
    } else {
        crate::security::audit::events::AuditEvent::FileCreated {
            path: resolved.clone(),
            uid,
        }
    });

    Ok(resolved)
}

pub fn validate_create_new_current(name: &str) -> Result<String, FsError> {
    let resolved = session::resolve_path(name);
    if !session::can_access_path(&resolved, true) {
        return Err(FsError::PermissionDenied);
    }

    let canonical = canonical_path(&resolved)?;
    if canonical == "/" {
        return Err(FsError::InvalidFilename);
    }
    if FILESYSTEM.lock().find(&canonical).is_some() {
        return Err(FsError::AlreadyExists);
    }
    let parent = parent_directory(&canonical);
    if !directory_exists(&parent) {
        return Err(FsError::FileNotFound);
    }
    Ok(canonical)
}

pub fn create_new_current(name: &str, data: &[u8]) -> Result<String, FsError> {
    let resolved = validate_create_new_current(name)?;
    let uid = session::current_uid();
    let permissions = default_permissions_for(&resolved, uid);
    FILESYSTEM
        .lock()
        .create_new_as(&resolved, data, uid, permissions, false)?;

    crate::security::audit::log_event(
        crate::security::audit::events::AuditEvent::FileCreated {
            path: resolved.clone(),
            uid,
        },
    );

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

    crate::security::audit::log_event(
        crate::security::audit::events::AuditEvent::FileDeleted {
            path: resolved.clone(),
            uid,
        },
    );

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

pub fn list_entries_current(path: Option<&str>) -> Result<(String, Vec<DirEntryView>), FsError> {
    let directory = path
        .map(session::resolve_path)
        .unwrap_or_else(session::current_cwd);
    if !session::can_access_path(&directory, false) {
        return Err(FsError::PermissionDenied);
    }
    if !directory_exists(&directory) {
        return Err(FsError::FileNotFound);
    }

    let uid = session::current_uid();
    let role = session::current_role();
    let filesystem = FILESYSTEM.lock();
    let mut entries = Vec::new();
    let mut seen_directories = Vec::<String>::new();

    for entry in filesystem.list() {
        if !entry.permissions.can_read(uid, role) {
            continue;
        }
        let Some(relative) = relative_child_path(&directory, &entry.name) else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }

        let mut components = relative.split('/');
        let Some(first_component) = components.next() else {
            continue;
        };
        if first_component == DIRECTORY_MARKER_NAME {
            continue;
        }

        if components.next().is_some() {
            if seen_directories.iter().any(|name| name == first_component) {
                continue;
            }
            seen_directories.push(first_component.into());
            let child_path = if directory == "/" {
                alloc::format!("/{first_component}")
            } else {
                alloc::format!("{directory}/{first_component}")
            };
            entries.push(DirEntryView {
                path: child_path,
                name: first_component.into(),
                is_dir: true,
                size: 0,
                modified_at: entry.modified_at,
                readonly: false,
                owner_uid: entry.owner_uid,
                permissions: entry.permissions,
            });
        } else {
            entries.push(DirEntryView {
                path: entry.name.clone(),
                name: first_component.into(),
                is_dir: false,
                size: entry.data.len(),
                modified_at: entry.modified_at,
                readonly: entry.readonly,
                owner_uid: entry.owner_uid,
                permissions: entry.permissions,
            });
        }
    }

    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok((directory, entries))
}

#[must_use]
pub fn directory_exists(path: &str) -> bool {
    let resolved = session::resolve_path(path);
    if resolved == "/" {
        return true;
    }

    let marker_path = directory_marker_path(&resolved);
    let filesystem = FILESYSTEM.lock();
    filesystem.list().iter().any(|entry| {
        entry.name == marker_path || entry.name.starts_with(&(resolved.clone() + "/"))
    })
}

pub fn change_directory(path: &str) -> Result<String, FsError> {
    let resolved = session::resolve_path(path);
    if !session::can_access_path(&resolved, false) {
        return Err(FsError::PermissionDenied);
    }
    if !directory_exists(&resolved) {
        return Err(FsError::FileNotFound);
    }
    session::set_cwd(&resolved);
    Ok(resolved)
}

pub fn mkdir_current(path: &str) -> Result<String, FsError> {
    let resolved = session::resolve_path(path);
    if !session::can_access_path(&resolved, true) {
        return Err(FsError::PermissionDenied);
    }

    let parent = parent_directory(&resolved);
    if !directory_exists(&parent) {
        return Err(FsError::FileNotFound);
    }

    let uid = session::current_uid();
    let role = session::current_role();
    let permissions = default_permissions_for(&resolved, uid);
    FILESYSTEM.lock().write_as(
        &directory_marker_path(&resolved),
        &[],
        uid,
        role,
        permissions,
    )?;
    Ok(resolved)
}

pub fn rmdir_current(path: &str) -> Result<String, FsError> {
    let resolved = session::resolve_path(path);
    if !session::can_access_path(&resolved, true) {
        return Err(FsError::PermissionDenied);
    }
    let marker = directory_marker_path(&resolved);
    let mut filesystem = FILESYSTEM.lock();
    let has_children = filesystem
        .list()
        .iter()
        .any(|entry| entry.name.starts_with(&(resolved.clone() + "/")) && entry.name != marker);
    if has_children {
        return Err(FsError::ReadOnly);
    }
    filesystem.delete_as(&marker, session::current_uid(), session::current_role())?;
    Ok(resolved)
}

pub fn copy_current(source: &str, destination: &str) -> Result<String, FsError> {
    let (_, data) = read_current(source)?;
    write_current(destination, &data)
}

pub fn move_current(source: &str, destination: &str) -> Result<String, FsError> {
    let (_, data) = read_current(source)?;
    let path = write_current(destination, &data)?;
    delete_current(source)?;
    Ok(path)
}

pub fn find_current(pattern: &str) -> Result<Vec<String>, FsError> {
    let query = pattern.trim();
    if query.is_empty() {
        return Err(FsError::InvalidFilename);
    }
    let filesystem = FILESYSTEM.lock();
    Ok(filesystem
        .list()
        .iter()
        .filter(|entry| !is_directory_marker(&entry.name))
        .filter(|entry| entry.name.contains(query) || basename(&entry.name).contains(query))
        .map(|entry| entry.name.clone())
        .collect())
}

pub fn grep_current(pattern: &str, path: &str) -> Result<Vec<GrepMatch>, FsError> {
    let (_, data) = read_current(path)?;
    let text = core::str::from_utf8(&data).map_err(|_| FsError::InvalidFilename)?;
    Ok(text
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(pattern))
        .map(|(index, line)| GrepMatch {
            line_number: index + 1,
            line: line.into(),
        })
        .collect())
}

pub fn head_current(path: &str, lines: usize) -> Result<Vec<String>, FsError> {
    let (_, data) = read_current(path)?;
    let text = core::str::from_utf8(&data).map_err(|_| FsError::InvalidFilename)?;
    Ok(text.lines().take(lines).map(String::from).collect())
}

pub fn tail_current(path: &str, lines: usize) -> Result<Vec<String>, FsError> {
    let (_, data) = read_current(path)?;
    let text = core::str::from_utf8(&data).map_err(|_| FsError::InvalidFilename)?;
    let collected: Vec<String> = text.lines().map(String::from).collect();
    let keep = lines.min(collected.len());
    Ok(collected[collected.len().saturating_sub(keep)..].to_vec())
}

pub fn wc_current(path: &str) -> Result<WordCount, FsError> {
    let (_, data) = read_current(path)?;
    let text = core::str::from_utf8(&data).map_err(|_| FsError::InvalidFilename)?;
    Ok(WordCount {
        lines: text.lines().count(),
        words: text.split_whitespace().count(),
        bytes: data.len(),
    })
}

pub fn diff_current(left: &str, right: &str) -> Result<Vec<String>, FsError> {
    let left_binding = read_current(left)?;
    let right_binding = read_current(right)?;
    let left_text = core::str::from_utf8(&left_binding.1).map_err(|_| FsError::InvalidFilename)?;
    let right_text = core::str::from_utf8(&right_binding.1).map_err(|_| FsError::InvalidFilename)?;
    let left_lines: Vec<&str> = left_text.lines().collect();
    let right_lines: Vec<&str> = right_text.lines().collect();
    let max = left_lines.len().max(right_lines.len());
    let mut output = Vec::new();
    for index in 0..max {
        match (left_lines.get(index), right_lines.get(index)) {
            (Some(left), Some(right)) if left == right => {}
            (Some(left), Some(right)) => {
                output.push(alloc::format!("- {}", left));
                output.push(alloc::format!("+ {}", right));
            }
            (Some(left), None) => output.push(alloc::format!("- {}", left)),
            (None, Some(right)) => output.push(alloc::format!("+ {}", right)),
            (None, None) => {}
        }
    }
    Ok(output)
}

pub fn sort_current(path: &str) -> Result<Vec<String>, FsError> {
    let (_, data) = read_current(path)?;
    let text = core::str::from_utf8(&data).map_err(|_| FsError::InvalidFilename)?;
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    lines.sort();
    Ok(lines)
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

pub fn canonicalize_warexec_path(
    path: &str,
    allow_directory_trailing_slash: bool,
) -> Result<String, FsError> {
    let trimmed = path.trim();
    if trimmed.is_empty() || !trimmed.starts_with('/') {
        return Err(FsError::InvalidFilename);
    }

    let has_trailing_slash = trimmed.len() > 1 && trimmed.ends_with('/');
    if has_trailing_slash && !allow_directory_trailing_slash {
        return Err(FsError::InvalidFilename);
    }

    let normalized = if allow_directory_trailing_slash && has_trailing_slash {
        trimmed.trim_end_matches('/')
    } else {
        trimmed
    };

    if normalized.len() > 1 && normalized.contains("//") {
        return Err(FsError::InvalidFilename);
    }

    if normalized == "/" {
        return Ok(String::from("/"));
    }

    let mut result = String::from("/");
    let mut first = true;
    for component in normalized.split('/').skip(1) {
        if component.is_empty() || component == "." || component == ".." {
            return Err(FsError::InvalidFilename);
        }
        validate_canonical_component(component)?;
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
        validate_canonical_component(component)?;
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

fn validate_canonical_component(component: &str) -> Result<(), FsError> {
    if component.len() > MAX_COMPONENT_LEN
        || !component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Err(FsError::InvalidFilename)
    } else {
        Ok(())
    }
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

const DIRECTORY_MARKER_NAME: &str = ".wardir";

fn directory_marker_path(path: &str) -> String {
    if path == "/" {
        String::from("/.wardir")
    } else {
        alloc::format!("{path}/{DIRECTORY_MARKER_NAME}")
    }
}

fn is_directory_marker(path: &str) -> bool {
    basename(path) == DIRECTORY_MARKER_NAME
}

fn relative_child_path<'a>(directory: &str, path: &'a str) -> Option<&'a str> {
    if directory == "/" {
        return path.strip_prefix('/');
    }

    let prefix = alloc::format!("{directory}/");
    path.strip_prefix(&prefix)
}
