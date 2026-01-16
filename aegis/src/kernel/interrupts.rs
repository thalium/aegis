use crate::{
    default_handler,
    kernel::interrupts::{
        cpu::{double_fault_handler, page_fault_handler},
        hardware::{PICS, keyboard_interrupt_handler, timer_interrupt_handler},
    },
};
use paste::paste;
use spin::Mutex;
use x86_64::{
    VirtAddr,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame},
};

mod cpu;
pub mod gdt;
pub mod hardware;

pub static IDT: Mutex<InterruptDescriptorTable> = Mutex::new(InterruptDescriptorTable::new());

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    PageFault = 14,
    Timer = PIC_1_OFFSET,
    Keyboard,
}

impl Into<u8> for InterruptIndex {
    fn into(self) -> u8 {
        self as u8
    }
}

impl Into<usize> for InterruptIndex {
    fn into(self) -> usize {
        self as u8 as usize
    }
}

/// Handles the interrupt descriptor table
pub struct InterruptManager;

impl InterruptManager {
    fn init_idt() {
        let mut idt = IDT.lock();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.device_not_available
            .set_handler_fn(device_not_available_handler);
        // idt.divide_error.set_handler_fn(divide_error_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        // idt.overflow.set_handler_fn(overflow_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);

        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
    }

    pub fn init() {
        gdt::init();

        InterruptManager::init_idt();

        // Initialize hardware interrupts
        {
            let mut idt = IDT.lock();
            idt[InterruptIndex::Timer as usize].set_handler_fn(timer_interrupt_handler);
            idt[InterruptIndex::Keyboard as usize].set_handler_fn(keyboard_interrupt_handler);
        }

        // Load the IDT with a static reference
        let idt_ref: &'static InterruptDescriptorTable = unsafe {
            // Safety: IDT lives for the entire program, so 'static is correct
            &*(&*IDT.lock() as *const InterruptDescriptorTable)
        };
        idt_ref.load();

        unsafe { PICS.lock().initialize() };
        x86_64::instructions::interrupts::enable();
    }

    /// Set the handler function for the IDT entry and sets the present bit.
    pub fn set_handler(idx: usize, handler: extern "x86-interrupt" fn(InterruptStackFrame)) {
        let mut idt = IDT.lock();
        idt[idx].set_handler_fn(handler);
    }

    /// Set the handler address for the IDT entry and sets the present bit
    pub unsafe fn set_handler_addr(idx: usize, handler: VirtAddr) {
        let mut idt = IDT.lock();

        unsafe {
            idt[idx].set_handler_addr(handler);
        }
    }
}

// Generates default handlers for
default_handler!(breakpoint);
default_handler!(device_not_available);
default_handler!(divide_error);
default_handler!(general_protection_fault);
default_handler!(invalid_opcode);
default_handler!(overflow);
