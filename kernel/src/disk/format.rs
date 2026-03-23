use alloc::vec;
use alloc::vec::Vec;
use core::mem::size_of;
use core::ptr;

use crate::arch::x86_64::interrupts;

use super::Disk;
use super::DiskError;

pub const WARFS_MAGIC: u32 = 0x5741_5246;
pub const WARFS_VERSION: u16 = 1;
pub const SECTOR_SIZE: usize = 512;
pub const BLOCK_SIZE: usize = 4096;
pub const SECTORS_PER_BLOCK: u64 = (BLOCK_SIZE / SECTOR_SIZE) as u64;
pub const MAX_FILES: usize = crate::fs::MAX_FILES;
pub const MAX_FILENAME: usize = crate::fs::MAX_PATH_LEN;
pub const FILE_TABLE_START_SECTOR: u64 = 1;
pub const DATA_START_SECTOR: u64 = FILE_TABLE_START_SECTOR + MAX_FILES as u64;
pub const FS_STATE_CLEAN: u8 = 0;
pub const FS_STATE_DIRTY: u8 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Superblock {
    pub magic: u32,
    pub version: u16,
    pub label: [u8; 32],
    pub total_sectors: u64,
    pub total_blocks: u32,
    pub used_blocks: u32,
    pub file_count: u16,
    pub block_size: u32,
    pub created_at: u64,
    pub last_mount: u64,
    pub mount_count: u32,
    pub state: u8,
    pub _reserved: [u8; 427],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DiskFileEntry {
    pub active: u8,
    pub name: [u8; MAX_FILENAME],
    pub size: u64,
    pub owner_uid: u16,
    pub perm_owner_read: u8,
    pub perm_owner_write: u8,
    pub perm_others_read: u8,
    pub perm_others_write: u8,
    pub readonly: u8,
    pub created_at: u64,
    pub modified_at: u64,
    pub start_block: u32,
    pub block_count: u32,
    pub _reserved: [u8; 368],
}

const _: [(); SECTOR_SIZE] = [(); size_of::<Superblock>()];
const _: [(); SECTOR_SIZE] = [(); size_of::<DiskFileEntry>()];

impl Superblock {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.magic == WARFS_MAGIC && self.version == WARFS_VERSION
    }

    #[must_use]
    pub fn new(total_sectors: u64) -> Self {
        let data_sectors = total_sectors.saturating_sub(DATA_START_SECTOR);
        let total_blocks = (data_sectors / SECTORS_PER_BLOCK) as u32;

        let mut label = [0u8; 32];
        let name = b"WarOS Disk";
        label[..name.len()].copy_from_slice(name);

        Self {
            magic: WARFS_MAGIC,
            version: WARFS_VERSION,
            label,
            total_sectors,
            total_blocks,
            used_blocks: 0,
            file_count: 0,
            block_size: BLOCK_SIZE as u32,
            created_at: interrupts::tick_count(),
            last_mount: 0,
            mount_count: 0,
            state: FS_STATE_CLEAN,
            _reserved: [0; 427],
        }
    }
}

impl DiskFileEntry {
    #[must_use]
    pub fn inactive() -> Self {
        Self {
            active: 0,
            name: [0; MAX_FILENAME],
            size: 0,
            owner_uid: 0,
            perm_owner_read: 0,
            perm_owner_write: 0,
            perm_others_read: 0,
            perm_others_write: 0,
            readonly: 0,
            created_at: 0,
            modified_at: 0,
            start_block: 0,
            block_count: 0,
            _reserved: [0; 368],
        }
    }
}

pub fn format_disk(disk: &mut Disk) -> Result<(), DiskError> {
    let superblock = Superblock::new(disk.capacity_sectors());
    write_superblock(disk, &superblock)?;

    let zero_sector = [0u8; SECTOR_SIZE];
    for sector in FILE_TABLE_START_SECTOR..DATA_START_SECTOR {
        disk.write_sectors(sector, 1, &zero_sector)?;
    }

    Ok(())
}

pub fn read_superblock(disk: &mut Disk) -> Result<Superblock, DiskError> {
    let mut buffer = [0u8; SECTOR_SIZE];
    disk.read_sectors(0, 1, &mut buffer)?;

    // SAFETY: `buffer` is exactly one sector and `Superblock` is verified to be one sector wide.
    let superblock = unsafe { ptr::read_unaligned(buffer.as_ptr().cast::<Superblock>()) };
    if !superblock.is_valid() {
        return Err(DiskError::FormatError);
    }

    Ok(superblock)
}

pub fn write_superblock(disk: &mut Disk, superblock: &Superblock) -> Result<(), DiskError> {
    // SAFETY: `Superblock` is sector-sized and plain old data with a stable `repr(C)` layout.
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (superblock as *const Superblock).cast::<u8>(),
            size_of::<Superblock>(),
        )
    };
    disk.write_sectors(0, 1, bytes)
}

pub fn mark_mounted(disk: &mut Disk) -> Result<Superblock, DiskError> {
    let mut superblock = read_superblock(disk)?;
    superblock.last_mount = interrupts::tick_count();
    superblock.mount_count = superblock.mount_count.saturating_add(1);
    superblock.state = FS_STATE_CLEAN;
    write_superblock(disk, &superblock)?;
    Ok(superblock)
}

pub fn set_superblock_state(disk: &mut Disk, state: u8) -> Result<(), DiskError> {
    let mut superblock = read_superblock(disk)?;
    superblock.state = state;
    write_superblock(disk, &superblock)
}

pub fn refresh_superblock(disk: &mut Disk, state: u8) -> Result<Superblock, DiskError> {
    let mut superblock = read_superblock(disk)?;
    superblock.used_blocks = count_used_blocks(disk)?;
    superblock.file_count = count_active_files(disk)?;
    superblock.state = state;
    write_superblock(disk, &superblock)?;
    Ok(superblock)
}

pub fn read_file_entry(disk: &mut Disk, index: usize) -> Result<DiskFileEntry, DiskError> {
    if index >= MAX_FILES {
        return Err(DiskError::OutOfBounds);
    }

    let mut buffer = [0u8; SECTOR_SIZE];
    let sector = FILE_TABLE_START_SECTOR + index as u64;
    disk.read_sectors(sector, 1, &mut buffer)?;

    // SAFETY: `buffer` is exactly one sector and `DiskFileEntry` is sector-sized.
    Ok(unsafe { ptr::read_unaligned(buffer.as_ptr().cast::<DiskFileEntry>()) })
}

pub fn write_file_entry(
    disk: &mut Disk,
    index: usize,
    entry: &DiskFileEntry,
) -> Result<(), DiskError> {
    if index >= MAX_FILES {
        return Err(DiskError::OutOfBounds);
    }

    // SAFETY: `DiskFileEntry` is sector-sized and plain old data with a stable `repr(C)` layout.
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (entry as *const DiskFileEntry).cast::<u8>(),
            size_of::<DiskFileEntry>(),
        )
    };
    let sector = FILE_TABLE_START_SECTOR + index as u64;
    disk.write_sectors(sector, 1, bytes)
}

pub fn entry_name(entry: &DiskFileEntry) -> Result<&str, DiskError> {
    let name_end = entry
        .name
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(MAX_FILENAME);
    core::str::from_utf8(&entry.name[..name_end]).map_err(|_| DiskError::CorruptedData)
}

#[must_use]
pub fn block_count_for_len(len: usize) -> u32 {
    if len == 0 {
        0
    } else {
        len.div_ceil(BLOCK_SIZE) as u32
    }
}

pub fn read_file_data(
    disk: &mut Disk,
    start_block: u32,
    block_count: u32,
    buffer: &mut [u8],
) -> Result<(), DiskError> {
    let total_sectors = block_count as u64 * SECTORS_PER_BLOCK;
    let start_sector = DATA_START_SECTOR + start_block as u64 * SECTORS_PER_BLOCK;
    disk.read_sectors(start_sector, total_sectors as u32, buffer)
}

pub fn write_file_data(
    disk: &mut Disk,
    start_block: u32,
    data: &[u8],
) -> Result<u32, DiskError> {
    let blocks_needed = block_count_for_len(data.len());
    if blocks_needed == 0 {
        return Ok(0);
    }

    let start_sector = DATA_START_SECTOR + start_block as u64 * SECTORS_PER_BLOCK;
    let mut padded = vec![0u8; blocks_needed as usize * BLOCK_SIZE];
    padded[..data.len()].copy_from_slice(data);
    disk.write_sectors(start_sector, (blocks_needed as u64 * SECTORS_PER_BLOCK) as u32, &padded)?;
    Ok(blocks_needed)
}

pub fn allocate_blocks(
    disk: &mut Disk,
    count: u32,
    ignore_index: Option<usize>,
) -> Result<u32, DiskError> {
    let superblock = read_superblock(disk)?;
    let mut used_ranges: Vec<(u32, u32)> = Vec::new();

    for index in 0..MAX_FILES {
        if Some(index) == ignore_index {
            continue;
        }

        let entry = read_file_entry(disk, index)?;
        if entry.active == 0 || entry.block_count == 0 {
            continue;
        }

        used_ranges.push((entry.start_block, entry.start_block + entry.block_count));
    }

    used_ranges.sort_unstable_by_key(|range| range.0);
    let mut candidate = 0u32;
    for (start, end) in used_ranges {
        if candidate.saturating_add(count) <= start {
            return Ok(candidate);
        }
        candidate = candidate.max(end);
    }

    if candidate.saturating_add(count) <= superblock.total_blocks {
        Ok(candidate)
    } else {
        Err(DiskError::DiskFull)
    }
}

pub fn count_active_files(disk: &mut Disk) -> Result<u16, DiskError> {
    let mut count = 0u16;
    for index in 0..MAX_FILES {
        if read_file_entry(disk, index)?.active != 0 {
            count = count.saturating_add(1);
        }
    }
    Ok(count)
}

pub fn count_used_blocks(disk: &mut Disk) -> Result<u32, DiskError> {
    let mut count = 0u32;
    for index in 0..MAX_FILES {
        let entry = read_file_entry(disk, index)?;
        if entry.active != 0 {
            count = count.saturating_add(entry.block_count);
        }
    }
    Ok(count)
}
