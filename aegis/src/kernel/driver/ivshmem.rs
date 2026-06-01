use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{Page, PhysFrame},
};

use crate::{
    kernel::{
        Kernel,
        driver::pci::{PciPorts, pci_scan},
    },
    println,
};

/// From https://www.qemu.org/docs/master/specs/ivshmem-spec.html
const VENDOR_ID: u16 = 0x1af4;
const DEVICE_ID: u16 = 0x1110;

/// Enumerates PCIs and attempts to find the ivshmem
fn find_ivshem() -> Option<(PhysAddr, usize)> {
    let devices = unsafe { pci_scan() };

    for dev in devices.iter() {
        if dev.vendor_id == VENDOR_ID && dev.device_id == DEVICE_ID {
            println!(
                "[*] Found ivshmem on PCI {:02x}:{:02x}.{:02x}",
                dev.bus, dev.device, dev.function
            );

            let mut bar = dev.bar(2);

            let mut ports = PciPorts::new();

            return Some((bar.base_address(&mut ports), bar.size(&mut ports)));
        }
    }
    None
}

/// Map a BAR into virtual memory using
fn map_bar(phys_base: PhysAddr, size: usize, virt_base: VirtAddr) {
    let mut offset = 0;

    while offset < size as u64 {
        let phys = phys_base + offset;
        let virt = virt_base + offset;

        let page = Page::containing_address(virt);
        let frame = PhysFrame::containing_address(phys);

        Kernel::get()
            .memory_manager
            .map_page_to_frame(page, frame)
            .expect("During IVSHMEM initialization, failed to map a page");

        offset += page.size();
    }
}

/// Maps the ivshmem to virtual memory starting at virt_base
pub fn init(virt_base: VirtAddr) -> usize {
    let (phys_base, size) = find_ivshem().expect("Did not find the ivshmem PCI");
    map_bar(phys_base, size, virt_base);
    size
}
