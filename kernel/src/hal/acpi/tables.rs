#[derive(Debug, Clone, Copy)]
pub struct Rsdp {
    pub revision: u8,
    pub rsdt_address: u32,
    pub xsdt_address: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct FadtInfo {
    pub pm1a_cnt_blk: u16,
    pub reset_reg_address: u64,
    pub reset_value: u8,
    pub dsdt_address: u64,
}
