use core::sync::atomic::{AtomicBool, Ordering};

use sha3::{Digest, Sha3_256};
use spin::Mutex;

static HAS_RDRAND: AtomicBool = AtomicBool::new(false);

pub struct EntropyPool {
    state: [u8; 32],
    entropy_bits: u32,
    generation: u64,
}

static POOL: Mutex<Option<EntropyPool>> = Mutex::new(None);

fn detect_rdrand() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        let result = core::arch::x86_64::__cpuid(1);
        result.ecx & (1 << 30) != 0
    }
    #[cfg(not(target_arch = "x86_64"))]
    false
}

fn rdrand_u64() -> Option<u64> {
    if !HAS_RDRAND.load(Ordering::Relaxed) {
        return None;
    }
    #[cfg(target_arch = "x86_64")]
    {
        let mut value: u64;
        let success: u8;
        unsafe {
            core::arch::asm!(
                "rdrand {val}",
                "setc {ok}",
                val = out(reg) value,
                ok = out(reg_byte) success,
            );
        }
        if success != 0 { Some(value) } else { None }
    }
    #[cfg(not(target_arch = "x86_64"))]
    None
}

impl EntropyPool {
    fn new() -> Self {
        let has_rdrand = detect_rdrand();
        HAS_RDRAND.store(has_rdrand, Ordering::Relaxed);

        let mut pool = Self {
            state: [0u8; 32],
            entropy_bits: 0,
            generation: 0,
        };

        // Seed from RDRAND if available
        if has_rdrand {
            for _ in 0..4 {
                if let Some(val) = rdrand_u64() {
                    pool.mix_bytes(&val.to_le_bytes());
                }
            }
            pool.entropy_bits = 256;
        }

        // Seed from PIT jitter
        let ticks = crate::arch::x86_64::interrupts::tick_count();
        pool.mix_bytes(&ticks.to_le_bytes());

        pool
    }

    fn mix_bytes(&mut self, data: &[u8]) {
        let mut hasher = Sha3_256::new();
        hasher.update(&self.state);
        hasher.update(data);
        let result = hasher.finalize();
        self.state.copy_from_slice(&result);
    }

    fn add_entropy_internal(&mut self, data: &[u8], bits: u32) {
        self.mix_bytes(data);
        self.entropy_bits = self.entropy_bits.saturating_add(bits).min(256);
    }

    fn generate(&mut self, buf: &mut [u8]) {
        let mut counter = self.generation;
        let mut offset = 0;

        while offset < buf.len() {
            let mut hasher = Sha3_256::new();
            hasher.update(&self.state);
            hasher.update(&counter.to_le_bytes());
            hasher.update(b"output");
            let block = hasher.finalize();

            let to_copy = (buf.len() - offset).min(32);
            buf[offset..offset + to_copy].copy_from_slice(&block[..to_copy]);
            offset += to_copy;
            counter += 1;
        }

        self.generation = counter;

        // Forward secrecy: update state so past outputs can't be reconstructed
        let mut hasher = Sha3_256::new();
        hasher.update(&self.state);
        hasher.update(&counter.to_le_bytes());
        hasher.update(b"reseed");
        let new_state = hasher.finalize();
        self.state.copy_from_slice(&new_state);
    }
}

pub fn init() {
    let pool = EntropyPool::new();
    *POOL.lock() = Some(pool);
}

pub fn has_rdrand() -> bool {
    HAS_RDRAND.load(Ordering::Relaxed)
}

pub fn entropy_bits() -> u32 {
    POOL.lock().as_ref().map_or(0, |p| p.entropy_bits)
}

pub fn add_entropy(data: &[u8]) {
    if let Some(pool) = POOL.lock().as_mut() {
        pool.add_entropy_internal(data, data.len() as u32);
    }
}

pub fn random_bytes(buf: &mut [u8]) {
    if let Some(pool) = POOL.lock().as_mut() {
        pool.generate(buf);
    }
}

pub fn random_u64() -> u64 {
    let mut buf = [0u8; 8];
    random_bytes(&mut buf);
    u64::from_le_bytes(buf)
}

/// Feed timer jitter into the pool (best-effort, non-blocking).
pub fn feed_timer_jitter(ticks: u64) {
    if let Some(mut guard) = POOL.try_lock() {
        if let Some(pool) = guard.as_mut() {
            let jitter = (ticks & 0xFF) as u8;
            pool.mix_bytes(&[jitter]);
        }
    }
}

/// Self-test: verify output is non-trivial.
pub fn self_test() -> bool {
    let mut buf = [0u8; 64];
    random_bytes(&mut buf);

    // Check not all zeros
    if buf.iter().all(|&b| b == 0) {
        return false;
    }
    // Check not all same byte
    if buf.iter().all(|&b| b == buf[0]) {
        return false;
    }
    true
}
