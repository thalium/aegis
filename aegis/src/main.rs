#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(aegis::kernel::tests::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

#[cfg(not(test))]
use core::panic::PanicInfo;

use aegis::{
    hlt_loop,
    kernel::{Kernel, driver::ivshmem},
    println,
    testing::{
        harness::{init_dataset, run_naked_test},
        shared_memory::{Region, SHARED_MEMORY_MANAGER},
        test_dataset::TestDataset,
    },
};
use alloc::boxed::Box;
use bootloader::{BootInfo, entry_point};
use libaegis::protocol::WRITE_REGION_OFFSET;
use x86_64::VirtAddr;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    Kernel::init(boot_info);

    let start = VirtAddr::new(0x_7777_7777_0000);
    let size = ivshmem::init(start);
    let shared_memory = Region::new(start, size);

    SHARED_MEMORY_MANAGER
        .lock()
        .init(shared_memory, WRITE_REGION_OFFSET);

    init_dataset(Box::new(TestDataset));

    println!("Starting testing...");

    run_naked_test();

    hlt_loop();
}

/// This function is called on panic.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{}", info);
    hlt_loop();
}
