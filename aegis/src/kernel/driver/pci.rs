use alloc::vec::Vec;
use x86_64::{PhysAddr, instructions::port::Port};

fn pci_to_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    (1 << 31)
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | ((offset as u32) & 0xFC)
}

/// Cached ports to avoid recreating
pub struct PciPorts {
    address: Port<u32>,
    data: Port<u32>,
}

impl PciPorts {
    const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
    const PCI_CONFIG_DATA: u16 = 0xCFC;

    pub fn new() -> Self {
        Self {
            address: Port::new(Self::PCI_CONFIG_ADDRESS),
            data: Port::new(Self::PCI_CONFIG_DATA),
        }
    }

    /// Writes data to a PCI
    fn read_u32(&mut self, bus: u8, device: u8, function: u8, offset: u8) -> u32 {
        unsafe {
            self.address
                .write(pci_to_address(bus, device, function, offset));
            self.data.read()
        }
    }

    /// Reads data from a PCI
    fn write_u32(&mut self, bus: u8, device: u8, function: u8, offset: u8, value: u32) {
        unsafe {
            self.address
                .write(pci_to_address(bus, device, function, offset));
            self.data.write(value)
        }
    }
}

/// A base address register
#[derive(Debug, Clone)]
pub struct BAR<'a> {
    device: &'a PciDevice,
    index: u8,
}

impl<'a> BAR<'a> {
    const ADDR_MASK: u32 = 0xFFFFFFF0;

    /// Reads the value of this BAR
    pub fn read(&self, ports: &mut PciPorts) -> u32 {
        self.device.read_bar(ports, self.index)
    }

    /// Writes to this BAR
    pub fn write(&mut self, ports: &mut PciPorts, value: u32) {
        self.device.write_bar(ports, self.index, value)
    }

    /// Verifies this BAR is memory
    pub fn is_memory(&self, ports: &mut PciPorts) -> bool {
        self.read(ports) & 0x1 == 0
    }

    /// Determines the physical address of the BAR region
    pub fn base_address(&self, ports: &mut PciPorts) -> PhysAddr {
        PhysAddr::new((self.read(ports) & Self::ADDR_MASK) as u64)
    }

    /// Determines the size of the BAR region
    pub fn size(&mut self, ports: &mut PciPorts) -> usize {
        // https://stackoverflow.com/questions/19006632/how-is-a-pci-pcie-bar-size-determined
        let old = self.read(ports);

        self.write(ports, 0xFFFFFFFF);

        let size = self.read(ports);

        // Restore the pci
        self.write(ports, old);

        (!(size & Self::ADDR_MASK) + 1) as usize
    }
}

/// A PCI device
#[repr(C)]
#[derive(Debug, Clone)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
}

impl PciDevice {
    /// Reads the contents of a BAR
    fn read_bar(&self, ports: &mut PciPorts, bar_index: u8) -> u32 {
        let offset = 0x10 + (bar_index as u8 * 4);
        ports.read_u32(self.bus, self.device, self.function, offset)
    }

    /// Writes content to a BAR
    fn write_bar(&self, ports: &mut PciPorts, bar_index: u8, value: u32) {
        let offset = 0x10 + (bar_index as u8 * 4);
        ports.write_u32(self.bus, self.device, self.function, offset, value);
    }

    pub fn bar(&self, index: u8) -> BAR<'_> {
        BAR {
            device: self,
            index,
        }
    }
}

/// Is this bus:device.function a PCI device
fn is_pci_device(ports: &mut PciPorts, bus: u8, device: u8, function: u8) -> Option<PciDevice> {
    let data = ports.read_u32(bus, device, function, 0x0);
    let vendor_id = (data & 0xFFFF) as u16;
    if vendor_id == 0xFFFF {
        return None;
    }

    let device_id = ((data >> 16) & 0xFFFF) as u16;

    Some(PciDevice {
        bus,
        device,
        function,
        vendor_id,
        device_id,
    })
}

/// Enumerates all PCI devices
pub unsafe fn pci_scan() -> Vec<PciDevice> {
    let mut ports = PciPorts::new();
    let mut devices = Vec::new();

    for bus in 0..=255 {
        for device in 0..32 {
            for function in 0..8 {
                if let Some(dev) = is_pci_device(&mut ports, bus, device, function) {
                    devices.push(dev);
                }
            }
        }
    }

    devices
}
