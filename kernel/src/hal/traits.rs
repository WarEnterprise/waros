#![allow(dead_code)]

use alloc::vec::Vec;

use crate::disk::DiskError;
use crate::net::NetError;

use super::device::{DeviceId, KeyboardLayout};

pub trait InputDriver: Send {
    fn name(&self) -> &str;
    fn device_id(&self) -> DeviceId;
    fn poll_keyboard(&mut self) -> Option<KeyEvent>;
    fn poll_mouse(&mut self) -> Option<MouseEvent>;
    fn set_layout(&mut self, layout: KeyboardLayout);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub scancode: u8,
    pub keycode: u8,
    pub pressed: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub dx: i16,
    pub dy: i16,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
    pub scroll_delta: i8,
}

pub trait NetworkDriver: Send {
    fn name(&self) -> &str;
    fn device_id(&self) -> DeviceId;
    fn mac_address(&self) -> [u8; 6];
    fn send_frame(&mut self, frame: &[u8]) -> Result<(), NetError>;
    fn recv_frame(&mut self) -> Option<Vec<u8>>;
    fn link_up(&self) -> bool;
    fn link_speed(&self) -> u32;
}

pub trait StorageDriver: Send {
    fn name(&self) -> &str;
    fn device_id(&self) -> DeviceId;
    fn capacity_sectors(&self) -> u64;
    fn sector_size(&self) -> u32;
    fn read_sectors(&mut self, sector: u64, count: u32, buf: &mut [u8]) -> Result<(), DiskError>;
    fn write_sectors(&mut self, sector: u64, count: u32, buf: &[u8]) -> Result<(), DiskError>;
    fn flush(&mut self) -> Result<(), DiskError>;
    fn is_removable(&self) -> bool;
}

pub trait DisplayDriver: Send {
    fn name(&self) -> &str;
    fn device_id(&self) -> DeviceId;
    fn resolution(&self) -> (u32, u32);
    fn bpp(&self) -> u8;
    fn framebuffer(&mut self) -> &mut [u8];
    fn stride(&self) -> usize;
}
