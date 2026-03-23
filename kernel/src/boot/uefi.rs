use bootloader_api::info::{FrameBuffer, FrameBufferInfo};

/// Returns the framebuffer layout reported by the UEFI-aware bootloader.
#[must_use]
pub fn framebuffer_info(framebuffer: &FrameBuffer) -> FrameBufferInfo {
    framebuffer.info()
}
