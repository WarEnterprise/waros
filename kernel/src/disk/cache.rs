use alloc::vec::Vec;

use super::format::SECTOR_SIZE;
use super::virtio_blk::VirtioBlk;
use super::DiskError;

#[derive(Clone)]
struct CacheEntry {
    sector: u64,
    data: [u8; SECTOR_SIZE],
    dirty: bool,
    generation: u64,
}

/// Small write-through sector cache for metadata-heavy filesystem traffic.
pub struct BlockCache {
    entries: Vec<CacheEntry>,
    capacity: usize,
    generation: u64,
}

impl BlockCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity,
            generation: 0,
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.generation = 0;
    }

    pub fn flush(&mut self, _device: &mut VirtioBlk) -> Result<(), DiskError> {
        for entry in &mut self.entries {
            entry.dirty = false;
        }
        Ok(())
    }

    pub fn read_sectors(
        &mut self,
        device: &mut VirtioBlk,
        sector: u64,
        count: u32,
        buffer: &mut [u8],
    ) -> Result<(), DiskError> {
        let required = count as usize * SECTOR_SIZE;
        if buffer.len() < required {
            return Err(DiskError::BufferTooSmall);
        }

        for index in 0..count as usize {
            let offset = index * SECTOR_SIZE;
            let mut sector_buffer = [0u8; SECTOR_SIZE];
            self.read_sector(device, sector + index as u64, &mut sector_buffer)?;
            buffer[offset..offset + SECTOR_SIZE].copy_from_slice(&sector_buffer);
        }

        Ok(())
    }

    pub fn write_sectors(
        &mut self,
        device: &mut VirtioBlk,
        sector: u64,
        count: u32,
        buffer: &[u8],
    ) -> Result<(), DiskError> {
        let required = count as usize * SECTOR_SIZE;
        if buffer.len() < required {
            return Err(DiskError::BufferTooSmall);
        }

        for index in 0..count as usize {
            let offset = index * SECTOR_SIZE;
            let mut sector_buffer = [0u8; SECTOR_SIZE];
            sector_buffer.copy_from_slice(&buffer[offset..offset + SECTOR_SIZE]);
            self.write_sector(device, sector + index as u64, &sector_buffer)?;
        }

        Ok(())
    }

    fn read_sector(
        &mut self,
        device: &mut VirtioBlk,
        sector: u64,
        buffer: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), DiskError> {
        let generation = self.next_generation();
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.sector == sector) {
            entry.generation = generation;
            buffer.copy_from_slice(&entry.data);
            return Ok(());
        }

        device.read_sectors(sector, 1, buffer)?;
        self.insert(sector, *buffer, false, generation);
        Ok(())
    }

    fn write_sector(
        &mut self,
        device: &mut VirtioBlk,
        sector: u64,
        data: &[u8; SECTOR_SIZE],
    ) -> Result<(), DiskError> {
        device.write_sectors(sector, 1, data)?;
        let generation = self.next_generation();
        self.insert(sector, *data, false, generation);
        Ok(())
    }

    fn insert(&mut self, sector: u64, data: [u8; SECTOR_SIZE], dirty: bool, generation: u64) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.sector == sector) {
            entry.data = data;
            entry.dirty = dirty;
            entry.generation = generation;
            return;
        }

        if self.entries.len() < self.capacity {
            self.entries.push(CacheEntry {
                sector,
                data,
                dirty,
                generation,
            });
            return;
        }

        if let Some((victim_index, _)) = self
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.generation)
        {
            self.entries[victim_index] = CacheEntry {
                sector,
                data,
                dirty,
                generation,
            };
        }
    }

    fn next_generation(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }
}
