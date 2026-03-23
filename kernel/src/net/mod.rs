use alloc::vec::Vec;

use spin::{Lazy, Mutex};

use crate::arch::x86_64::port;

const MAGIC: &[u8; 4] = b"WNET";
const COM2_PORT: u16 = 0x2F8;

pub static NET: Lazy<Mutex<NetInterface>> = Lazy::new(|| Mutex::new(NetInterface::new(COM2_PORT)));

/// Wire protocol message types for the serial link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Ping = 0x01,
    Pong = 0x02,
    CircuitData = 0x10,
    MeasurementResult = 0x11,
    Text = 0x20,
}

/// Parsed incoming message frame.
#[derive(Debug, Clone)]
pub struct Message {
    pub msg_type: MessageType,
    pub payload: Vec<u8>,
}

/// Serial-link failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    PayloadTooLarge,
    InvalidFrame,
    NotInitialized,
}

/// Minimal serial networking interface on COM2.
pub struct NetInterface {
    serial_port: u16,
    rx_buffer: Vec<u8>,
    initialized: bool,
}

impl NetInterface {
    #[must_use]
    pub fn new(serial_port: u16) -> Self {
        Self {
            serial_port,
            rx_buffer: Vec::new(),
            initialized: false,
        }
    }

    pub fn init(&mut self) {
        let port = self.serial_port;
        port::outb(port + 1, 0x00);
        port::outb(port + 3, 0x80);
        port::outb(port, 0x03);
        port::outb(port + 1, 0x00);
        port::outb(port + 3, 0x03);
        port::outb(port + 2, 0xC7);
        port::outb(port + 4, 0x0B);
        self.initialized = true;
    }

    pub fn send(&self, msg_type: MessageType, payload: &[u8]) -> Result<(), NetError> {
        if !self.initialized {
            return Err(NetError::NotInitialized);
        }
        let length = u16::try_from(payload.len()).map_err(|_| NetError::PayloadTooLarge)?;
        let checksum = crc16(msg_type as u8, length, payload);

        for byte in MAGIC {
            self.write_byte(*byte);
        }
        self.write_byte(msg_type as u8);
        self.write_byte((length & 0xFF) as u8);
        self.write_byte((length >> 8) as u8);
        for byte in payload {
            self.write_byte(*byte);
        }
        self.write_byte((checksum & 0xFF) as u8);
        self.write_byte((checksum >> 8) as u8);
        Ok(())
    }

    pub fn receive(&mut self) -> Option<Message> {
        while let Some(byte) = self.read_byte() {
            self.rx_buffer.push(byte);
        }

        let start = self
            .rx_buffer
            .windows(MAGIC.len())
            .position(|window| window == MAGIC)?;
        if start > 0 {
            self.rx_buffer.drain(..start);
        }

        if self.rx_buffer.len() < 9 {
            return None;
        }

        let msg_type = self.rx_buffer[4];
        let length = u16::from(self.rx_buffer[5]) | (u16::from(self.rx_buffer[6]) << 8);
        let frame_len = 4usize + 1 + 2 + usize::from(length) + 2;
        if self.rx_buffer.len() < frame_len {
            return None;
        }

        let payload = self.rx_buffer[7..(7 + usize::from(length))].to_vec();
        let checksum_offset = 7 + usize::from(length);
        let checksum = u16::from(self.rx_buffer[checksum_offset])
            | (u16::from(self.rx_buffer[checksum_offset + 1]) << 8);
        self.rx_buffer.drain(..frame_len);

        let kind = MessageType::try_from(msg_type).ok()?;
        if checksum != crc16(msg_type, length, &payload) {
            return None;
        }

        if kind == MessageType::Ping {
            let _ = self.send(MessageType::Pong, b"pong");
        }

        Some(Message {
            msg_type: kind,
            payload,
        })
    }

    pub fn send_circuit(&self, qasm: &str) -> Result<(), NetError> {
        self.send(MessageType::CircuitData, qasm.as_bytes())
    }

    pub fn send_text(&self, text: &str) -> Result<(), NetError> {
        self.send(MessageType::Text, text.as_bytes())
    }

    #[must_use]
    pub fn status(&self) -> &'static str {
        if self.initialized {
            "COM2 active"
        } else {
            "COM2 offline"
        }
    }

    fn write_byte(&self, byte: u8) {
        while port::inb(self.serial_port + 5) & 0x20 == 0 {}
        port::outb(self.serial_port, byte);
    }

    fn read_byte(&self) -> Option<u8> {
        if port::inb(self.serial_port + 5) & 0x01 == 0 {
            None
        } else {
            Some(port::inb(self.serial_port))
        }
    }
}

impl TryFrom<u8> for MessageType {
    type Error = NetError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::Ping),
            0x02 => Ok(Self::Pong),
            0x10 => Ok(Self::CircuitData),
            0x11 => Ok(Self::MeasurementResult),
            0x20 => Ok(Self::Text),
            _ => Err(NetError::InvalidFrame),
        }
    }
}

/// Initialize the COM2 serial link.
pub fn init() {
    NET.lock().init();
}

/// Send a text message over COM2.
pub fn send_text(text: &str) -> Result<(), NetError> {
    NET.lock().send_text(text)
}

/// Send a QASM or textual circuit payload over COM2.
pub fn send_circuit(qasm: &str) -> Result<(), NetError> {
    NET.lock().send_circuit(qasm)
}

/// Poll for one received message.
#[must_use]
pub fn receive() -> Option<Message> {
    NET.lock().receive()
}

/// Human-readable interface status.
#[must_use]
pub fn status() -> &'static str {
    NET.lock().status()
}

fn crc16(msg_type: u8, length: u16, payload: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    crc = crc16_byte(crc, msg_type);
    crc = crc16_byte(crc, (length & 0xFF) as u8);
    crc = crc16_byte(crc, (length >> 8) as u8);
    for byte in payload {
        crc = crc16_byte(crc, *byte);
    }
    crc
}

fn crc16_byte(mut crc: u16, byte: u8) -> u16 {
    crc ^= u16::from(byte);
    for _ in 0..8 {
        if crc & 1 == 1 {
            crc = (crc >> 1) ^ 0xA001;
        } else {
            crc >>= 1;
        }
    }
    crc
}
