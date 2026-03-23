pub mod cache;
pub mod format;
pub mod persist;
pub mod virtio_blk;

use alloc::vec::Vec;
use core::fmt;

use spin::{Lazy, Mutex};

use crate::fs::{FileEntry, FsError, WarFS, FILESYSTEM};

use self::cache::BlockCache;
use self::format::{read_superblock, FS_STATE_CLEAN};
use self::persist::{disk_usage, load_filesystem, save_entry, sync_filesystem};
use self::virtio_blk::{find_virtio_blk, VirtioBlk};

const DEFAULT_CACHE_SECTORS: usize = 64;

pub static DISK: Lazy<Mutex<Option<Disk>>> = Lazy::new(|| Mutex::new(None));

pub struct Disk {
    device: VirtioBlk,
    cache: BlockCache,
}

#[derive(Debug, Clone, Copy)]
pub struct DiskInitReport {
    pub size_mb: u64,
    pub version: u16,
    pub loaded_files: usize,
    pub formatted: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct DiskStatus {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub io_base: u16,
    pub capacity_sectors: u64,
    pub disk_size: u64,
    pub version: u16,
    pub total_blocks: u32,
    pub used_blocks: u32,
    pub file_count: u16,
    pub state: u8,
}

#[derive(Debug, Clone, Copy)]
pub enum MountMode {
    RamOnly,
    DiskBacked { version: u16, disk_size: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskError {
    DeviceNotFound,
    UnsupportedDevice(&'static str),
    InitFailed(&'static str),
    OutOfMemory,
    OutOfBounds,
    BufferTooSmall,
    DeviceError,
    FormatError,
    CorruptedData,
    DiskFull,
    FileNotFound,
    FilesystemError(FsError),
}

impl Disk {
    fn new(device: VirtioBlk) -> Self {
        Self {
            device,
            cache: BlockCache::new(DEFAULT_CACHE_SECTORS),
        }
    }

    pub fn read_sectors(
        &mut self,
        sector: u64,
        count: u32,
        buffer: &mut [u8],
    ) -> Result<(), DiskError> {
        self.cache.read_sectors(&mut self.device, sector, count, buffer)
    }

    pub fn write_sectors(
        &mut self,
        sector: u64,
        count: u32,
        buffer: &[u8],
    ) -> Result<(), DiskError> {
        self.cache.write_sectors(&mut self.device, sector, count, buffer)
    }

    pub fn flush(&mut self) -> Result<(), DiskError> {
        self.cache.flush(&mut self.device)
    }

    pub fn reset_cache(&mut self) {
        self.cache.clear();
    }

    #[must_use]
    pub fn capacity_sectors(&self) -> u64 {
        self.device.capacity_sectors
    }

    #[must_use]
    pub fn disk_size(&self) -> u64 {
        self.device.disk_size
    }

    #[must_use]
    pub fn io_base(&self) -> u16 {
        self.device.io_base()
    }

    #[must_use]
    pub fn pci_device(&self) -> crate::net::pci::PciDevice {
        self.device.pci_device()
    }
}

impl fmt::Display for DiskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceNotFound => formatter.write_str("no virtio-blk device found"),
            Self::UnsupportedDevice(reason) => write!(formatter, "unsupported disk device: {reason}"),
            Self::InitFailed(reason) => write!(formatter, "disk initialization failed: {reason}"),
            Self::OutOfMemory => formatter.write_str("disk DMA allocation failed"),
            Self::OutOfBounds => formatter.write_str("disk sector request is out of bounds"),
            Self::BufferTooSmall => formatter.write_str("disk buffer is smaller than the request"),
            Self::DeviceError => formatter.write_str("virtio-blk device reported an I/O error"),
            Self::FormatError => formatter.write_str("disk does not contain a valid WarFS volume"),
            Self::CorruptedData => formatter.write_str("disk metadata is corrupted"),
            Self::DiskFull => formatter.write_str("disk is full"),
            Self::FileNotFound => formatter.write_str("file not found on disk"),
            Self::FilesystemError(error) => write!(formatter, "filesystem error: {error}"),
        }
    }
}

impl From<FsError> for DiskError {
    fn from(error: FsError) -> Self {
        Self::FilesystemError(error)
    }
}

pub fn init(filesystem: &mut WarFS) -> Result<Option<DiskInitReport>, DiskError> {
    let Some(pci) = find_virtio_blk() else {
        return Ok(None);
    };

    let mut disk = Disk::new(VirtioBlk::init(pci)?);
    let mut formatted = false;
    match read_superblock(&mut disk) {
        Ok(_) => {}
        Err(DiskError::FormatError) => {
            format::format_disk(&mut disk)?;
            disk.reset_cache();
            formatted = true;
        }
        Err(error) => return Err(error),
    }

    let mounted = format::mark_mounted(&mut disk)?;
    let loaded_files = if formatted {
        0
    } else {
        load_filesystem(&mut disk, filesystem)?
    };

    let report = DiskInitReport {
        size_mb: disk.disk_size() / (1024 * 1024),
        version: mounted.version,
        loaded_files,
        formatted,
    };

    *DISK.lock() = Some(disk);
    Ok(Some(report))
}

#[must_use]
pub fn is_available() -> bool {
    DISK.lock().is_some()
}

#[must_use]
pub fn mount_mode() -> MountMode {
    let mut guard = DISK.lock();
    let Some(disk) = guard.as_mut() else {
        return MountMode::RamOnly;
    };

    match read_superblock(disk) {
        Ok(superblock) => MountMode::DiskBacked {
            version: superblock.version,
            disk_size: disk.disk_size(),
        },
        Err(_) => MountMode::DiskBacked {
            version: 0,
            disk_size: disk.disk_size(),
        },
    }
}

pub fn disk_status() -> Result<Option<DiskStatus>, DiskError> {
    let mut guard = DISK.lock();
    let Some(disk) = guard.as_mut() else {
        return Ok(None);
    };

    let superblock = read_superblock(disk)?;
    let pci = disk.pci_device();
    let (file_count, used_blocks) = disk_usage(disk)?;
    Ok(Some(DiskStatus {
        bus: pci.bus,
        device: pci.device,
        function: pci.function,
        io_base: disk.io_base(),
        capacity_sectors: disk.capacity_sectors(),
        disk_size: disk.disk_size(),
        version: superblock.version,
        total_blocks: superblock.total_blocks,
        used_blocks,
        file_count,
        state: superblock.state,
    }))
}

pub fn maybe_sync_file_entry(entry: &FileEntry) {
    if !is_persistable_path(&entry.name) {
        return;
    }

    if let Some(disk) = DISK.lock().as_mut() {
        let _ = save_entry(disk, entry);
    }
}

pub fn maybe_delete_path(path: &str) {
    if !is_persistable_path(path) {
        return;
    }

    if let Some(disk) = DISK.lock().as_mut() {
        let _ = persist::delete_file_from_disk(disk, path);
    }
}

pub fn sync_all() -> Result<usize, DiskError> {
    let snapshot: Vec<FileEntry> = {
        let filesystem = FILESYSTEM.lock();
        filesystem.list().to_vec()
    };

    let mut guard = DISK.lock();
    let Some(disk) = guard.as_mut() else {
        return Ok(0);
    };

    let synced = sync_filesystem(disk, &snapshot)?;
    disk.flush()?;
    Ok(synced)
}

pub fn format_active() -> Result<bool, DiskError> {
    let mut guard = DISK.lock();
    let Some(disk) = guard.as_mut() else {
        return Ok(false);
    };

    format::format_disk(disk)?;
    disk.reset_cache();
    let _ = format::mark_mounted(disk)?;
    let _ = format::refresh_superblock(disk, FS_STATE_CLEAN)?;
    Ok(true)
}

#[must_use]
pub fn state_label(state: u8) -> &'static str {
    if state == FS_STATE_CLEAN {
        "clean"
    } else {
        "dirty"
    }
}

#[must_use]
pub fn is_persistable_path(path: &str) -> bool {
    !matches!(path, "/readme.txt" | "/sysinfo.txt")
}
