pub mod entropy;
pub mod file_encryption;
pub mod qkd;

pub fn init() {
    entropy::init();
}
