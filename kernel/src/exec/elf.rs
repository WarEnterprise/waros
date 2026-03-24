use alloc::vec::Vec;

use super::process::SegmentFlags;
use super::ExecError;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Elf64Header {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Elf64ProgramHeader {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

pub const PT_LOAD: u32 = 1;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

#[derive(Debug, Clone)]
pub struct ElfInfo {
    pub entry_point: u64,
    pub segments: Vec<ElfSegment>,
    pub is_pie: bool,
}

#[derive(Debug, Clone)]
pub struct ElfSegment {
    pub vaddr: u64,
    pub memsz: u64,
    pub filesz: u64,
    pub offset: u64,
    pub flags: SegmentFlags,
    pub align: u64,
}

pub fn parse_elf(data: &[u8]) -> Result<ElfInfo, ExecError> {
    if data.len() < core::mem::size_of::<Elf64Header>() {
        return Err(ExecError::TooSmall);
    }

    let header = read_struct::<Elf64Header>(data, 0)?;
    if &header.e_ident[0..4] != b"\x7fELF" {
        return Err(ExecError::NotElf);
    }
    if header.e_ident[4] != 2 {
        return Err(ExecError::Not64Bit);
    }
    if header.e_ident[5] != 1 {
        return Err(ExecError::NotLittleEndian);
    }
    if header.e_machine != 0x3E {
        return Err(ExecError::WrongArchitecture);
    }
    if !matches!(header.e_type, 2 | 3) {
        return Err(ExecError::NotExecutable);
    }
    if header.e_phentsize as usize != core::mem::size_of::<Elf64ProgramHeader>() {
        return Err(ExecError::InvalidProgramHeader);
    }

    let mut segments = Vec::new();
    let ph_offset = header.e_phoff as usize;
    let ph_size = header.e_phentsize as usize;
    let ph_count = header.e_phnum as usize;

    for index in 0..ph_count {
        let offset = ph_offset + index.saturating_mul(ph_size);
        let program = read_struct::<Elf64ProgramHeader>(data, offset)?;
        if program.p_type != PT_LOAD {
            continue;
        }
        if program.p_memsz < program.p_filesz {
            return Err(ExecError::SegmentOverflow);
        }
        let file_end = program.p_offset.saturating_add(program.p_filesz) as usize;
        if file_end > data.len() {
            return Err(ExecError::SegmentOverflow);
        }

        let flags = SegmentFlags::from_bits_truncate(
            (if program.p_flags & PF_R != 0 {
                SegmentFlags::READ.bits()
            } else {
                0
            }) | (if program.p_flags & PF_W != 0 {
                SegmentFlags::WRITE.bits()
            } else {
                0
            }) | (if program.p_flags & PF_X != 0 {
                SegmentFlags::EXECUTE.bits()
            } else {
                0
            }),
        );
        segments.push(ElfSegment {
            vaddr: program.p_vaddr,
            memsz: program.p_memsz,
            filesz: program.p_filesz,
            offset: program.p_offset,
            flags,
            align: program.p_align,
        });
    }

    if segments.is_empty() {
        return Err(ExecError::NoLoadableSegments);
    }

    Ok(ElfInfo {
        entry_point: header.e_entry,
        segments,
        is_pie: header.e_type == 3,
    })
}

fn read_struct<T: Copy>(data: &[u8], offset: usize) -> Result<T, ExecError> {
    let size = core::mem::size_of::<T>();
    if offset.saturating_add(size) > data.len() {
        return Err(ExecError::InvalidProgramHeader);
    }

    // SAFETY: Bounds are validated above and ELF structures are plain POD types.
    let value = unsafe { (data.as_ptr().add(offset) as *const T).read_unaligned() };
    Ok(value)
}
