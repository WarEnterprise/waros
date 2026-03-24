#[derive(Debug, Clone, Copy)]
pub enum Signal {
    Term,
    Kill,
    Stop,
    Continue,
}
