use core::ops::{Deref, DerefMut};

use bootloader::BootInfo;
use spin::{Mutex, MutexGuard};

use crate::kernel::{interrupts::InterruptManager, memory::PhysicallyMappedMemoryManager};

pub mod driver;
pub mod interrupts;
pub mod memory;
pub mod qemu;
pub mod tests;

static KERNEL: Mutex<Option<Kernel>> = Mutex::new(None);

/// Our kernel
pub struct Kernel {
    pub memory_manager: PhysicallyMappedMemoryManager,
}

impl Kernel {
    pub fn init(boot_info: &'static BootInfo) {
        InterruptManager::init();

        let mut kernel = KERNEL.lock();
        *kernel = Some(Self {
            memory_manager: PhysicallyMappedMemoryManager::new(boot_info),
        });
    }

    pub fn get() -> KernelGuard<'static> {
        KernelGuard {
            inner: KERNEL.lock(),
        }
    }
}

/// Custom wrapper around a locked kernel
pub struct KernelGuard<'a> {
    inner: MutexGuard<'a, Option<Kernel>>,
}

impl<'a> Deref for KernelGuard<'a> {
    type Target = Kernel;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().expect("Kernel not initialized")
    }
}

impl<'a> DerefMut for KernelGuard<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().expect("Kernel not initialized")
    }
}
