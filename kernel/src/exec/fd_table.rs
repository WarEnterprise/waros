use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub enum FileHandleAccess {
    ReadOnly,
    CreateWrite,
}

#[derive(Debug, Clone)]
pub struct FileHandle {
    pub path: String,
    pub offset: usize,
    pub access: FileHandleAccess,
    pub staged_data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct DirectoryEntryHandle {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone)]
pub struct DirectoryHandle {
    pub path: String,
    pub entries: Vec<DirectoryEntryHandle>,
    pub cursor: usize,
}

#[derive(Debug, Clone)]
pub enum DescriptorTarget {
    Stdin,
    Stdout,
    Stderr,
    File(FileHandle),
    Directory(DirectoryHandle),
    Socket(u32),
    Pipe(u32),
}

#[derive(Debug, Clone)]
pub struct FileDescriptor {
    pub fd: u32,
    pub target: DescriptorTarget,
}

#[derive(Debug, Clone)]
pub struct FileDescriptorTable {
    entries: Vec<Option<FileDescriptor>>,
}

impl FileDescriptorTable {
    #[must_use]
    pub fn new_with_stdio() -> Self {
        Self {
            entries: vec![
                Some(FileDescriptor {
                    fd: 0,
                    target: DescriptorTarget::Stdin,
                }),
                Some(FileDescriptor {
                    fd: 1,
                    target: DescriptorTarget::Stdout,
                }),
                Some(FileDescriptor {
                    fd: 2,
                    target: DescriptorTarget::Stderr,
                }),
            ],
        }
    }

    pub fn insert(&mut self, target: DescriptorTarget) -> u32 {
        if let Some((index, slot)) = self
            .entries
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        {
            let fd = index as u32;
            *slot = Some(FileDescriptor { fd, target });
            return fd;
        }

        let fd = self.entries.len() as u32;
        self.entries.push(Some(FileDescriptor { fd, target }));
        fd
    }

    #[must_use]
    pub fn get(&self, fd: u32) -> Option<&FileDescriptor> {
        self.entries.get(fd as usize).and_then(Option::as_ref)
    }

    pub fn get_mut(&mut self, fd: u32) -> Option<&mut FileDescriptor> {
        self.entries.get_mut(fd as usize).and_then(Option::as_mut)
    }

    pub fn close(&mut self, fd: u32) -> bool {
        if let Some(entry) = self.entries.get_mut(fd as usize) {
            *entry = None;
            true
        } else {
            false
        }
    }
}
