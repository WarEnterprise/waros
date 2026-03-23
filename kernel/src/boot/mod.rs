use bootloader_api::BootInfo;
use x86_64::VirtAddr;

pub mod uefi;

/// Information extracted from the bootloader's boot info for early kernel init.
pub struct BootContext<'a> {
    pub framebuffer: &'a mut bootloader_api::info::FrameBuffer,
    pub memory_regions: &'a bootloader_api::info::MemoryRegions,
    pub physical_memory_offset: VirtAddr,
}

/// Translate bootloader-provided state into a simpler kernel-owned view.
pub fn bootstrap(boot_info: &'static mut BootInfo) -> Result<BootContext<'static>, &'static str> {
    let framebuffer = boot_info
        .framebuffer
        .as_mut()
        .ok_or("bootloader did not provide a framebuffer")?;
    let physical_memory_offset = boot_info
        .physical_memory_offset
        .as_ref()
        .copied()
        .ok_or("bootloader physical memory mapping is unavailable")?;

    Ok(BootContext {
        framebuffer,
        memory_regions: &boot_info.memory_regions,
        physical_memory_offset: VirtAddr::new(physical_memory_offset),
    })
}
