use bootloader::BootInfo;
use x86_64::{
    VirtAddr,
    structures::paging::{
        FrameAllocator, Mapper, OffsetPageTable, Page, PageTableFlags, PhysFrame, Size4KiB,
        mapper::MapToError,
    },
};

use crate::kernel::memory::paging::BootInfoFrameAllocator;

pub mod allocator;
pub mod paging;

pub struct PhysicallyMappedMemoryManager {
    mapper: OffsetPageTable<'static>,
    frame_allocator: BootInfoFrameAllocator,
}

impl PhysicallyMappedMemoryManager {
    pub fn new(boot_info: &'static BootInfo) -> Self {
        let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);

        let mut mapper = unsafe { paging::init(phys_mem_offset) };
        let mut frame_allocator = unsafe { BootInfoFrameAllocator::init(&boot_info.memory_map) };

        // Initialize heap
        allocator::init_heap(&mut mapper, &mut frame_allocator)
            .expect("heap initialization failed");

        Self {
            mapper,
            frame_allocator,
        }
    }

    /// Returns the mapper
    pub fn mapper(&mut self) -> &mut dyn Mapper<Size4KiB> {
        &mut self.mapper
    }

    /// Creates and maps a page starting at a virtual address
    pub fn map_addr(&mut self, addr: VirtAddr) -> Result<Page, MapToError<Size4KiB>> {
        let page = Page::<Size4KiB>::containing_address(addr);
        self.map_page(page)
    }

    /// Maps a given page
    pub fn map_page(&mut self, page: Page) -> Result<Page, MapToError<Size4KiB>> {
        let frame = self
            .frame_allocator
            .allocate_frame()
            .ok_or(MapToError::FrameAllocationFailed)?;

        self.map_page_to_frame(page, frame)?;

        Ok(page)
    }

    /// Maps a given page to aframe
    pub fn map_page_to_frame(
        &mut self,
        page: Page,
        frame: PhysFrame,
    ) -> Result<Page, MapToError<Size4KiB>> {
        unsafe {
            self.mapper
                .map_to(
                    page,
                    frame,
                    PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
                    &mut self.frame_allocator,
                )?
                .flush()
        };

        Ok(page)
    }
}
