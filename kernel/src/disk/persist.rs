use alloc::vec;

use crate::auth::FilePermissions;
use crate::fs::{FileEntry, WarFS};

use super::format::{
    allocate_blocks, block_count_for_len, count_active_files, count_used_blocks, entry_name,
    read_file_data, read_file_entry, refresh_superblock, set_superblock_state, write_file_data,
    write_file_entry, DiskFileEntry, FS_STATE_CLEAN, FS_STATE_DIRTY, MAX_FILENAME, MAX_FILES,
    BLOCK_SIZE,
};
use super::{is_persistable_path, Disk, DiskError};

pub fn load_filesystem(disk: &mut Disk, fs: &mut WarFS) -> Result<usize, DiskError> {
    let mut loaded = 0usize;

    for index in 0..MAX_FILES {
        let entry = read_file_entry(disk, index)?;
        if entry.active == 0 {
            continue;
        }

        let name = entry_name(&entry)?;
        if !is_persistable_path(name) {
            continue;
        }

        let mut data = vec![0u8; entry.size as usize];
        if entry.block_count != 0 && entry.size != 0 {
            let mut block_buffer = vec![0u8; entry.block_count as usize * BLOCK_SIZE];
            read_file_data(disk, entry.start_block, entry.block_count, &mut block_buffer)?;
            data.copy_from_slice(&block_buffer[..entry.size as usize]);
        }

        let permissions = FilePermissions {
            owner_uid: entry.owner_uid,
            owner_read: entry.perm_owner_read != 0,
            owner_write: entry.perm_owner_write != 0,
            others_read: entry.perm_others_read != 0,
            others_write: entry.perm_others_write != 0,
        };

        fs.load_file_from_disk(
            name,
            &data,
            entry.owner_uid,
            permissions,
            entry.readonly != 0,
            entry.created_at,
            entry.modified_at,
        )?;
        loaded += 1;
    }

    Ok(loaded)
}

pub fn save_entry(disk: &mut Disk, entry: &FileEntry) -> Result<(), DiskError> {
    set_superblock_state(disk, FS_STATE_DIRTY)?;
    upsert_file_entry(
        disk,
        &entry.name,
        &entry.data,
        entry.owner_uid,
        &entry.permissions,
        entry.readonly,
        entry.created_at,
        entry.modified_at,
    )?;
    let _ = refresh_superblock(disk, FS_STATE_CLEAN)?;
    Ok(())
}

pub fn delete_file_from_disk(disk: &mut Disk, name: &str) -> Result<(), DiskError> {
    set_superblock_state(disk, FS_STATE_DIRTY)?;

    let mut found = false;
    for index in 0..MAX_FILES {
        let entry = read_file_entry(disk, index)?;
        if entry.active == 0 {
            continue;
        }

        if entry_name(&entry)? == name {
            write_file_entry(disk, index, &DiskFileEntry::inactive())?;
            found = true;
            break;
        }
    }

    if !found {
        return Err(DiskError::FileNotFound);
    }

    let _ = refresh_superblock(disk, FS_STATE_CLEAN)?;
    Ok(())
}

pub fn sync_filesystem(disk: &mut Disk, files: &[FileEntry]) -> Result<usize, DiskError> {
    set_superblock_state(disk, FS_STATE_DIRTY)?;

    for index in 0..MAX_FILES {
        let entry = read_file_entry(disk, index)?;
        if entry.active == 0 {
            continue;
        }

        let name = entry_name(&entry)?;
        let should_keep = files
            .iter()
            .any(|file| is_persistable_path(&file.name) && file.name == name);

        if !should_keep {
            write_file_entry(disk, index, &DiskFileEntry::inactive())?;
        }
    }

    let mut synced = 0usize;
    for file in files {
        if !is_persistable_path(&file.name) {
            continue;
        }

        upsert_file_entry(
            disk,
            &file.name,
            &file.data,
            file.owner_uid,
            &file.permissions,
            file.readonly,
            file.created_at,
            file.modified_at,
        )?;
        synced += 1;
    }

    let _ = refresh_superblock(disk, FS_STATE_CLEAN)?;
    Ok(synced)
}

pub fn disk_usage(disk: &mut Disk) -> Result<(u16, u32), DiskError> {
    Ok((count_active_files(disk)?, count_used_blocks(disk)?))
}

fn upsert_file_entry(
    disk: &mut Disk,
    name: &str,
    data: &[u8],
    owner_uid: u16,
    permissions: &FilePermissions,
    readonly: bool,
    created_at: u64,
    modified_at: u64,
) -> Result<(), DiskError> {
    let mut target_index = None;
    let mut target_entry = None;
    let mut empty_index = None;

    for index in 0..MAX_FILES {
        let entry = read_file_entry(disk, index)?;
        if entry.active != 0 {
            if entry_name(&entry)? == name {
                target_index = Some(index);
                target_entry = Some(entry);
                break;
            }
        } else if empty_index.is_none() {
            empty_index = Some(index);
        }
    }

    let index = target_index.or(empty_index).ok_or(DiskError::DiskFull)?;
    let existing = target_entry;
    let blocks_needed = block_count_for_len(data.len());
    let start_block = if blocks_needed == 0 {
        0
    } else if let Some(existing) = existing {
        if existing.block_count >= blocks_needed {
            existing.start_block
        } else {
            allocate_blocks(disk, blocks_needed, Some(index))?
        }
    } else {
        allocate_blocks(disk, blocks_needed, None)?
    };

    if blocks_needed != 0 {
        write_file_data(disk, start_block, data)?;
    }

    let mut entry = DiskFileEntry {
        active: 1,
        name: [0u8; MAX_FILENAME],
        size: data.len() as u64,
        owner_uid,
        perm_owner_read: if permissions.owner_read { 1 } else { 0 },
        perm_owner_write: if permissions.owner_write { 1 } else { 0 },
        perm_others_read: if permissions.others_read { 1 } else { 0 },
        perm_others_write: if permissions.others_write { 1 } else { 0 },
        readonly: if readonly { 1 } else { 0 },
        created_at,
        modified_at,
        start_block,
        block_count: blocks_needed,
        _reserved: [0; 368],
    };

    let name_bytes = name.as_bytes();
    let copy_len = name_bytes.len().min(MAX_FILENAME.saturating_sub(1));
    entry.name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
    write_file_entry(disk, index, &entry)?;
    Ok(())
}
