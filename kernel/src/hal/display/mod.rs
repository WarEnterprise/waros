use alloc::format;
use alloc::string::String;

use bootloader_api::info::{FrameBufferInfo, PixelFormat as BootPixelFormat};

use crate::hal::DEVICES;

use super::device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceId, DeviceInfo, DeviceStatus,
    DisplayCapabilities, DriverState, PixelFormat,
};

pub fn register_framebuffer(info: FrameBufferInfo) -> DeviceId {
    DEVICES.lock().register_or_update(
        DeviceInfo {
            name: format!(
                "Framebuffer {}x{} {}bpp",
                info.width,
                info.height,
                info.bytes_per_pixel * 8
            ),
            category: DeviceCategory::Display,
            bus: BusLocation::Platform,
            vendor_id: 0,
            product_id: 0,
            capabilities: DeviceCapabilities::Display(DisplayCapabilities {
                width: info.width as u32,
                height: info.height as u32,
                bpp: (info.bytes_per_pixel * 8) as u8,
                pixel_format: convert_pixel_format(info.pixel_format),
            }),
        },
        DriverState::Loaded(String::from("framebuffer")),
        DeviceStatus::Active,
    )
}

fn convert_pixel_format(format: BootPixelFormat) -> PixelFormat {
    match format {
        BootPixelFormat::Bgr => PixelFormat::Bgr,
        BootPixelFormat::Rgb => PixelFormat::Rgb,
        BootPixelFormat::U8 => PixelFormat::Mono,
        BootPixelFormat::Unknown { .. } => PixelFormat::Unknown,
        _ => PixelFormat::Unknown,
    }
}
