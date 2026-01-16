pub mod harness;
pub mod shared_memory;
pub mod test_dataset;

pub const SHARED_MEMORY_START: usize = 0x_7777_7777_0000;
pub const SHARED_MEMORY_SIZE: usize = 0x1_000_000;
