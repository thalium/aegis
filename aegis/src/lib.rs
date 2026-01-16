#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(crate::kernel::tests::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![feature(abi_x86_interrupt)]

#[cfg(test)]
use bootloader::{BootInfo, entry_point};

#[cfg(test)]
entry_point!(test_kernel_main);

pub mod kernel;
pub mod testing;

extern crate alloc;

#[cfg(test)]
fn test_kernel_main(boot_info: &'static BootInfo) -> ! {
    use crate::kernel::Kernel;

    Kernel::init(boot_info);
    test_main();
    hlt_loop();
}

pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}
