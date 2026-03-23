use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use arrayvec::ArrayString;
use spin::{Lazy, Mutex};

use crate::arch::x86_64::interrupts;
use crate::memory;
use crate::{boot_complete_ms, BUILD_DATE, KERNEL_VERSION};

pub const MAX_FILES: usize = 64;
pub const MAX_FILE_SIZE: usize = 64 * 1024;
pub const TOTAL_CAPACITY: usize = 4 * 1024 * 1024;

pub static FILESYSTEM: Lazy<Mutex<WarFS>> = Lazy::new(|| Mutex::new(WarFS::new()));

/// Flat in-memory filesystem backed by the kernel heap.
pub struct WarFS {
    files: Vec<FileEntry>,
    max_files: usize,
}

/// One file entry stored in the `WarFS` heap-backed catalogue.
#[derive(Clone)]
pub struct FileEntry {
    pub name: ArrayString<32>,
    pub data: Vec<u8>,
    pub created_at: u64,
    pub modified_at: u64,
    pub readonly: bool,
}

/// Filesystem operation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    FileNotFound,
    FilesystemFull,
    FilenameTooLong,
    FileTooLarge,
    ReadOnly,
    InvalidFilename,
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
        self.write_internal(name, data, false)
    }

    pub fn write_readonly(&mut self, name: &str, data: &[u8]) -> Result<(), FsError> {
        self.write_internal(name, data, true)
    }

    pub fn touch(&mut self, name: &str) -> Result<(), FsError> {
        self.write(name, &[])
    }

    pub fn read(&self, name: &str) -> Result<&[u8], FsError> {
        let canonical = canonical_name(name)?;
        self.files
            .iter()
            .find(|entry| entry.name == canonical)
            .map(|entry| entry.data.as_slice())
            .ok_or(FsError::FileNotFound)
    }

    pub fn delete(&mut self, name: &str) -> Result<(), FsError> {
        let canonical = canonical_name(name)?;
        let Some(index) = self.files.iter().position(|entry| entry.name == canonical) else {
            return Err(FsError::FileNotFound);
        };
        if self.files[index].readonly {
            return Err(FsError::ReadOnly);
        }
        self.files.remove(index);
        Ok(())
    }

    #[must_use]
    pub fn list(&self) -> &[FileEntry] {
        &self.files
    }

    #[must_use]
    pub fn exists(&self, name: &str) -> bool {
        canonical_name(name)
            .ok()
            .is_some_and(|canonical| self.files.iter().any(|entry| entry.name == canonical))
    }

    #[must_use]
    pub fn free_space(&self) -> usize {
        TOTAL_CAPACITY.saturating_sub(self.used_space())
    }

    #[must_use]
    pub fn used_space(&self) -> usize {
        self.files.iter().map(|entry| entry.data.len()).sum()
    }

    #[must_use]
    pub fn stat(&self, name: &str) -> Result<&FileEntry, FsError> {
        let canonical = canonical_name(name)?;
        self.files
            .iter()
            .find(|entry| entry.name == canonical)
            .ok_or(FsError::FileNotFound)
    }

    fn write_internal(&mut self, name: &str, data: &[u8], readonly: bool) -> Result<(), FsError> {
        if data.len() > MAX_FILE_SIZE {
            return Err(FsError::FileTooLarge);
        }

        let canonical = canonical_name(name)?;
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

        if let Some(entry) = self.files.iter_mut().find(|entry| entry.name == canonical) {
            if entry.readonly {
                return Err(FsError::ReadOnly);
            }
            entry.data.clear();
            entry.data.extend_from_slice(data);
            entry.modified_at = now;
            entry.readonly = readonly;
            return Ok(());
        }

        if self.files.len() >= self.max_files {
            return Err(FsError::FilesystemFull);
        }

        self.files.push(FileEntry {
            name: canonical,
            data: data.to_vec(),
            created_at: now,
            modified_at: now,
            readonly,
        });
        self.files.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(())
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
        }
    }
}

/// Initialize the global `WarFS` instance.
pub fn init() {
    drop(FILESYSTEM.lock());
}

/// Populate the built-in system files after boot completes.
pub fn seed_system_files() -> Result<(), FsError> {
    let readme = concat!(
        "WarOS v0.1.0 - Quantum-Classical Hybrid Operating System\n",
        "War Enterprise (c) 2026\n\n",
        "This is a RAM-based filesystem. Files are lost on reboot.\n",
        "Use 'help fs' for filesystem commands.\n",
        "Use 'help quantum' for quantum computing commands.\n\n",
        "github.com/WarEnterprise/waros\n",
    );

    let stats = memory::stats();
    let sysinfo = format!(
        "WarOS v{KERNEL_VERSION}\n\
Kernel: waros-kernel {KERNEL_VERSION}\n\
Built: {BUILD_DATE}\n\
Boot time: {} ms\n\
RAM: {} MiB ({} frames)\n\
Heap: 4 MiB\n\
Quantum: StateVector (18 qubits max)\n\
Filesystem: {} files max, {} KiB max per file\n",
        boot_complete_ms(),
        (stats.total_frames * 4) / 1024,
        stats.total_frames,
        MAX_FILES,
        MAX_FILE_SIZE / 1024
    );

    let mut filesystem = FILESYSTEM.lock();
    filesystem.write_readonly("/readme.txt", readme.as_bytes())?;
    filesystem.write_readonly("/sysinfo.txt", sysinfo.as_bytes())?;
    Ok(())
}

/// Format PIT ticks as `HH:MM:SS`.
#[must_use]
pub fn format_timestamp(ticks: u64) -> String {
    let seconds = ticks / 100;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn canonical_name(name: &str) -> Result<ArrayString<32>, FsError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(FsError::InvalidFilename);
    }

    let stripped = trimmed.strip_prefix('/').unwrap_or(trimmed);
    if stripped.is_empty() || stripped.contains('/') {
        return Err(FsError::InvalidFilename);
    }
    if !stripped
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(FsError::InvalidFilename);
    }
    if stripped.len() > 31 {
        return Err(FsError::FilenameTooLong);
    }

    let mut canonical = ArrayString::<32>::new();
    canonical.push('/');
    canonical.push_str(stripped);
    Ok(canonical)
}
