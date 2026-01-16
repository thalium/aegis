use alloc::{boxed::Box, format, string::String, vec::Vec};
use libaegis::{cpu::*, testcase::TestCase, wrap};
use paste::paste;
use raw_cpuid::CpuId;
use x86_64::{
    VirtAddr,
    registers::{
        control::{Cr4, Cr4Flags},
        xcontrol::{XCr0, XCr0Flags},
    },
    structures::{
        idt::{InterruptStackFrame, PageFaultErrorCode},
        paging::{Page, PageTableFlags, Size4KiB, mapper::MapToError},
    },
};

use crate::{
    kernel::{
        Kernel,
        interrupts::{
            IDT, InterruptIndex, InterruptManager,
            gdt::DOUBLE_FAULT_IST_INDEX,
            hardware::{PICS, pit_set_interval},
        },
    },
    println,
};
use spin::Mutex;

pub const ICE_START: usize = 0x_6666_6666_0000;
pub const FIRE_START: usize = 0x_6666_6666_1000;

/// Creates the ice and fire pages
///
/// The ice page is a writable page.
/// The fire page has no permissions.
///
/// Instructions should be placed at the end of the ICE page, causing a page
/// fault when the fire page is accessed.
///
/// 0x_6666_6666_0000 -> +---------------+
///                      |               |
///                      |    ICE page   |
///                      |     (RWX)     |
/// 0x_6666_6666_0FFF -> +---------------+
///                      |               |
///                      |   FIRE page   |
///                      |      (0)      |
///                      +---------------+
///
pub fn create_ice_and_fire(kernel: &mut Kernel) -> Result<(), MapToError<Size4KiB>> {
    // ------------------------
    // Ice page: R/W/X
    // ------------------------
    let ice_start = VirtAddr::new(ICE_START as u64);
    let ice_page = kernel.memory_manager.map_addr(ice_start)?;

    // ------------------------
    // Map fire page normally first
    // ------------------------
    let fire_page: Page = ice_page + 1;
    assert_eq!(fire_page.start_address(), VirtAddr::new(FIRE_START as u64));
    let fire_page = kernel.memory_manager.map_addr(fire_page.start_address())?;

    // ------------------------
    // Remove all permissions from the fire page
    // ------------------------
    unsafe {
        kernel
            .memory_manager
            .mapper()
            .update_flags(fire_page, PageTableFlags::empty())
            .expect("While creating the fire page, unable to clear flags")
            .flush();
    }

    Ok(())
}

/// This needs to be aligned
pub const CPU_DUMP_START: usize = 0x_5555_0000_0000;

/// Create the CPU state huge page
/// TODO: this won't work with concurency
pub fn create_cpu_dump_pages(kernel: &mut Kernel) -> Result<(), MapToError<Size4KiB>> {
    let start = VirtAddr::new(CPU_DUMP_START as u64);

    let mut page = Page::<Size4KiB>::containing_address(start);

    for _ in 0..4 {
        kernel.memory_manager.map_addr(page.start_address())?;
        page = page + 1;
    }

    Ok(())
}

/// Adds an instruction at the end of the ICE page and returns the address of this instruction
pub fn set_ice_instruction(insn_buffer: &[u8]) -> VirtAddr {
    let start = FIRE_START - insn_buffer.len() - 1;

    // Safety: ICE page needs to be initialized
    unsafe {
        core::ptr::copy_nonoverlapping(insn_buffer.as_ptr(), start as *mut u8, insn_buffer.len());
    }

    VirtAddr::new(start as u64)
}

pub type TestId = usize;

pub trait Dataset: Sync + Send {
    /// Returns the next test of the dataset
    fn next(&self) -> TestCase;

    fn after_test(&mut self, id: TestId, state: &CpuState);
}

pub static DATASET: Mutex<Option<Box<dyn Dataset>>> = Mutex::new(None);
pub static TEST_ID: Mutex<TestId> = Mutex::new(0);

pub static TEST_INSN: Mutex<Vec<u8>> = Mutex::new(Vec::new());

fn print_last_insn() {
    let s = TEST_INSN
        .lock()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();

    println!("Last instruction: {}", s);
}

// The custom page_fault_handler
fn page_fault_handler(
    state: &CpuState,
    stack_frame: &InterruptStackFrame,
    error_code: PageFaultErrorCode,
    rip: &mut u64,
) {
    if error_code != PageFaultErrorCode::INSTRUCTION_FETCH {
        print_last_insn();
        panic!("This looks like a real page fault: {:?}", stack_frame);
    }

    // x86_64::instructions::interrupts::enable();

    landing_pad(state);
    *rip = run_naked_test as *const () as u64;
}

wrap!(raw_page_fault_handler, page_fault_handler);

// The custom timer_handler
pub extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer as u8);
    }
}

// Double fault handler
pub fn double_fault_handler(
    _state: &CpuState,
    stack_frame: &InterruptStackFrame,
    _error_code: PageFaultErrorCode,
    rip: &mut u64,
) {
    // Outside of the expected area
    if *rip < ICE_START as u64 || *rip > FIRE_START as u64 {
        print_last_insn();
        panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
    }

    println!("! DOUBLE FAULT");

    // Ignore this test so no landing pad

    *rip = run_naked_test as *const () as u64;
}

wrap!(raw_double_fault_handler, double_fault_handler);

pub fn init_dataset(dataset: Box<dyn Dataset>) {
    // Verifies the CPU features
    verify_cpu();

    // Enable the xsave instruction
    unsafe {
        Cr4::update(|flags| *flags = flags.union(Cr4Flags::OSXSAVE));
        XCr0::write(XCr0Flags::X87 | XCr0Flags::SSE | XCr0Flags::AVX);
    }

    // Sets the timer handler
    InterruptManager::set_handler(InterruptIndex::Timer as usize, timer_interrupt_handler);

    // Set the timer frequency
    pit_set_interval(1);

    // Sets the page fault handler
    unsafe {
        IDT.lock()
            .page_fault
            .set_handler_addr(VirtAddr::new(raw_page_fault_handler as *const () as u64));
    }

    // Sets the double fault handler
    unsafe {
        IDT.lock()
            .double_fault
            .set_handler_addr(VirtAddr::new(raw_double_fault_handler as *const () as u64))
            .set_stack_index(DOUBLE_FAULT_IST_INDEX);
    }

    // Initialize the run
    let mut kernel = Kernel::get();
    println!("Initializing ice and fire...");
    create_ice_and_fire(&mut kernel).expect("Failed to create ice or fire pages");
    println!("Initializing CPU dump page...");
    create_cpu_dump_pages(&mut kernel).expect("Failed to create the cpu dump pages");

    // Sets the dataset
    *DATASET.lock() = Some(dataset);
}

/// We want to ensure that our function's stack is cleared before we jump to the test
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn run_naked_test() {
    // Pop the return address too
    core::arch::naked_asm!("mov rdi, rsp", "call {run_test_}", run_test_ = sym run_test_)
}

#[doc(hidden)]
pub extern "C" fn run_test_(rsp: u64) -> ! {
    let mut test = {
        let dataset = DATASET.lock();

        match dataset.as_ref() {
            Some(d) => d.next(),
            None => panic!("No dataset"),
        }
    };

    *TEST_ID.lock() = test.id;

    // Write the instruction

    // Save the instruction for error logging
    *TEST_INSN.lock() = test.insn[..test.size as usize]
        .iter()
        .copied()
        .collect::<Vec<u8>>();

    let rip = set_ice_instruction(&test.insn[..test.size as usize]).as_u64();
    test.state.rip = rip;
    test.state.gpr.rsp = rsp;

    // Sets the registers
    // jump to rip
    unsafe {
        core::arch::asm!(
        // Move CPUState pointer into rdi
        "mov rdi, {CPU_DUMP_ADDR}",

        // Restore AVX registers
        "mov eax, 0xFFFFFFFF",
        "mov edx, 0xFFFFFFFF",
        "xrstor [rdi + {OFFSET_AVX}]",

        // Restore most general purpose registers
        "mov rax, [rdi + {OFFSET_RAX}]",
        "mov rbx, [rdi + {OFFSET_RBX}]",
        "mov rcx, [rdi + {OFFSET_RCX}]",
        "mov rdx, [rdi + {OFFSET_RDX}]",
        "mov rsi, [rdi + {OFFSET_RSI}]",
        "mov rbp, [rdi + {OFFSET_RBP}]",
        "mov rsp, [rdi + {OFFSET_RSP}]",
        "mov r8,  [rdi + {OFFSET_R8}]",
        "mov r9,  [rdi + {OFFSET_R9}]",
        "mov r10, [rdi + {OFFSET_R10}]",
        "mov r11, [rdi + {OFFSET_R11}]",
        "mov r12, [rdi + {OFFSET_R12}]",
        "mov r13, [rdi + {OFFSET_R13}]",
        "mov r14, [rdi + {OFFSET_R14}]",
        // "mov r15, [rdi + {OFFSET_R15}]",

        // Restore MMX registers
        "movq mm0, [rdi + {OFFSET_MM0}]",
        "movq mm1, [rdi + {OFFSET_MM1}]",
        "movq mm2, [rdi + {OFFSET_MM2}]",
        "movq mm3, [rdi + {OFFSET_MM3}]",
        "movq mm4, [rdi + {OFFSET_MM4}]",
        "movq mm5, [rdi + {OFFSET_MM5}]",
        "movq mm6, [rdi + {OFFSET_MM6}]",
        "movq mm7, [rdi + {OFFSET_MM7}]",

        // Restore segment registers
        // "mov cs, [rdi + {OFFSET_CS}]",
        "mov ds, [rdi + {OFFSET_DS}]",
        "mov es, [rdi + {OFFSET_ES}]",
        "mov fs, [rdi + {OFFSET_FS}]",
        "mov gs, [rdi + {OFFSET_GS}]",
        "mov ss, [rdi + {OFFSET_SS}]",

        // Restore flags
        "mov r15, [rdi + {OFFSET_FLAGS}]",
        "push r15",
        "popfq",

        // Restore rdi
        "mov r15, rdi",
        "mov rdi, [r15 + {OFFSET_RDI}]",

        "mov r15, [r15 + {OFFSET_RIP}]",
        "jmp r15",


        CPU_DUMP_ADDR = in(reg) &test.state,
        OFFSET_RAX = const OFFSET_RAX,
        OFFSET_RBX = const OFFSET_RBX,
        OFFSET_RCX = const OFFSET_RCX,
        OFFSET_RDX = const OFFSET_RDX,
        OFFSET_RSI = const OFFSET_RSI,
        OFFSET_RDI = const OFFSET_RDI,
        OFFSET_RIP = const OFFSET_RIP,
        OFFSET_RBP = const OFFSET_RBP,
        OFFSET_RSP = const OFFSET_RSP,
        OFFSET_R8 = const OFFSET_R8,
        OFFSET_R9 = const OFFSET_R9,
        OFFSET_R10 = const OFFSET_R10,
        OFFSET_R11 = const OFFSET_R11,
        OFFSET_R12 = const OFFSET_R12,
        OFFSET_R13 = const OFFSET_R13,
        OFFSET_R14 = const OFFSET_R14,
        // OFFSET_R15 = const OFFSET_R15,
        OFFSET_FLAGS = const OFFSET_FLAGS,
        // OFFSET_CS = const OFFSET_CS,
        OFFSET_DS = const OFFSET_DS,
        OFFSET_ES = const OFFSET_ES,
        OFFSET_FS = const OFFSET_FS,
        OFFSET_GS = const OFFSET_GS,
        OFFSET_SS = const OFFSET_SS,
        OFFSET_MM0 = const OFFSET_MM0,
        OFFSET_MM1 = const OFFSET_MM1,
        OFFSET_MM2 = const OFFSET_MM2,
        OFFSET_MM3 = const OFFSET_MM3,
        OFFSET_MM4 = const OFFSET_MM4,
        OFFSET_MM5 = const OFFSET_MM5,
        OFFSET_MM6 = const OFFSET_MM6,
        OFFSET_MM7 = const OFFSET_MM7,
        OFFSET_AVX = const OFFSET_AVX,
                options(noreturn)
            );
    };
}

pub fn landing_pad(state: &CpuState) {
    {
        // run some logic
        match DATASET.lock().as_mut() {
            Some(d) => d.after_test(*TEST_ID.lock(), state),
            None => panic!("No dataset"),
        };
    }
}

fn verify_cpu() {
    let cpuid = CpuId::new();

    let f1 = cpuid.get_feature_info().unwrap();
    let has_sse = f1.has_sse();
    let has_sse2 = f1.has_sse2();
    let has_avx = f1.has_avx();
    let has_xsave = f1.has_xsave();

    println!("SSE: {}", has_sse && has_sse2);
    println!("AVX usable: {}", has_avx);
    println!("XSAVE: {}", has_xsave);

    // TODO: do something if this is not the case
    assert!(has_sse && has_sse2 && has_avx && has_xsave);
}
