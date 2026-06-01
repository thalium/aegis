use alloc::{boxed::Box, format, string::String, vec::Vec};
use libaegis::{
    cpu::*,
    testcase::{ExceptionInfo, ExceptionVector, TestCase},
};
use raw_cpuid::CpuId;
use x86_64::{
    VirtAddr,
    instructions::hlt,
    registers::{
        control::{Cr0, Cr0Flags, Cr2, Cr4, Cr4Flags},
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
        interrupts::{IDT, UNIFIED_HANDLER, gdt::DOUBLE_FAULT_IST_INDEX},
    },
    println,
};
use spin::Mutex;

pub const ICE_START: usize = 0x_6666_6666_0000;
pub const FIRE_START: usize = 0x_6666_6666_1000;

pub const TEST_STACK_START: usize = 0x_6666_6600_0000;
pub const TEST_STACK: usize = 0x_6666_6600_5fff;

pub const MEM_ADDR: usize = 0x_6666_6601_0100;

/// This needs to be aligned
pub const CPU_DUMP_START: usize = 0x_5555_0000_0000;
pub static DATASET: Mutex<Option<Box<dyn Dataset>>> = Mutex::new(None);
pub static TEST_ID: Mutex<TestId> = Mutex::new(0);

pub static TEST_INSN: Mutex<Vec<u8>> = Mutex::new(Vec::new());

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
    // Stack page: R/W
    // ------------------------
    // Allocate 5 pages for the test stack
    let mut addr = TEST_STACK_START;
    while addr < TEST_STACK {
        let stack_page_start = VirtAddr::new(addr as u64);
        kernel.memory_manager.map_addr(stack_page_start)?;
        addr += 0x1000;
    }

    let stack_start = VirtAddr::new((MEM_ADDR as u64) & 0xffff_ffff_f000);
    kernel.memory_manager.map_addr(stack_start)?;

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

/// Create the CPU state huge page
pub fn create_cpu_dump_pages(kernel: &mut Kernel) -> Result<(), MapToError<Size4KiB>> {
    let start = VirtAddr::new(CPU_DUMP_START as u64);

    let mut page = Page::<Size4KiB>::containing_address(start);

    for _ in 0..4 {
        kernel.memory_manager.map_addr(page.start_address())?;
        page += 1;
    }

    Ok(())
}

/// Adds an instruction at the end of the ICE page and returns the address of this instruction
pub fn set_ice_instruction(insn_buffer: &[u8]) -> VirtAddr {
    let start = FIRE_START - insn_buffer.len();

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

    fn after_test(&mut self, id: TestId, state: &CpuState, exception: Option<ExceptionInfo>);
}

fn print_last_insn() {
    let s = TEST_INSN
        .lock()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();

    println!("Last instruction: {}", s);
}

pub fn exception_handler(
    state: &mut CpuState,
    stack_frame: &mut InterruptStackFrame,
    error_code: Option<u64>,
    vector: ExceptionVector,
) {
    let mut is_exception = true;

    if vector == ExceptionVector::Page {
        let error_code = PageFaultErrorCode::from_bits_truncate(error_code.unwrap_or(0));
        if state.rip < ICE_START as u64 || state.rip > (FIRE_START as u64 + 0x1000) {
            print_last_insn();

            let fault_addr = Cr2::read();
            let fault_rip = stack_frame.instruction_pointer.as_u64();

            println!("PAGE FAULT");
            println!("CR2 / accessed addr = {:#x}", fault_addr.as_u64());
            println!("RIP / faulting instr = {:#x}", fault_rip);
            println!("error code = {:?}", error_code);
            println!("saved rip = {:#x}", state.rip);

            panic!(
                "Unexpected page fault while not in test area: {:?}",
                stack_frame
            );
        }

        if error_code == PageFaultErrorCode::INSTRUCTION_FETCH {
            is_exception = false;
        } else {
            print_last_insn();
            println!(
                "Attempted to access address {:#x} {error_code:#?}",
                Cr2::read().as_u64()
            );
            hlt();
        }
    }
    // println!("EXCEPTION {vector}({error_code:?})");

    // Outside of the expected area
    if vector != ExceptionVector::Page
        && (state.rip < ICE_START as u64 || state.rip > (FIRE_START as u64 + 0x1000))
    {
        print_last_insn();
        panic!("EXCEPTION {vector}({error_code:?}): \n{stack_frame:#?}");
    }

    let exception = if is_exception { Some(vector) } else { None };

    landing_pad(state, exception);

    // Give the test a stack
    unsafe {
        stack_frame.as_mut().update(|f| {
            f.stack_pointer = VirtAddr::new(TEST_STACK as u64);
            f.instruction_pointer = VirtAddr::new(run_test as *const () as u64);
            // Clear flags
            f.cpu_flags = 0;
        });
    }
}

// Double fault handler
#[unsafe(no_mangle)]
pub extern "x86-interrupt" fn double_fault_handler_testing(
    stack_frame: InterruptStackFrame,
    code: u64,
) -> ! {
    // Outside of the expected area
    print_last_insn();
    panic!("EXCEPTION: DOUBLE FAULT({code})\n{:#?}", stack_frame);
}

pub fn init_dataset(dataset: Box<dyn Dataset>) {
    // Enable the xsave instruction
    unsafe {
        Cr4::update(|flags| {
            *flags = flags.union(Cr4Flags::OSXSAVE | Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE)
        });
        XCr0::write(
            XCr0Flags::X87
                | XCr0Flags::SSE
                | XCr0Flags::AVX
                | XCr0Flags::OPMASK
                | XCr0Flags::ZMM_HI256
                | XCr0Flags::HI16_ZMM,
        );
    }

    // Verifies the CPU features
    verify_cpu();

    // Sets the generic handler
    unsafe {
        UNIFIED_HANDLER = exception_handler;
    }

    // Sets the double fault handler
    unsafe {
        IDT.lock()
            .double_fault
            .set_handler_fn(double_fault_handler_testing)
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

#[unsafe(no_mangle)]
pub extern "C" fn run_test() -> ! {
    let mut test = {
        let dataset = DATASET.lock();

        match dataset.as_ref() {
            Some(d) => d.next(),
            None => panic!("No dataset"),
        }
    };

    *TEST_ID.lock() = test.id;
    // Save the instruction for error logging

    // if *TEST_INSN.lock() != test.insn[..test.size as usize] {
    //     println!("New instruction: {:02x?}", &test.insn[..test.size as usize]);
    // }

    *TEST_INSN.lock() = test.insn[..test.size as usize].to_vec();

    let rip = set_ice_instruction(&test.insn[..test.size as usize]).as_u64();
    test.state.rip = rip;

    unsafe {
        *(MEM_ADDR as *mut u64) = test.state.mem0;
    }

    // test.state.flags.0 &= !INTERRUPT_FLAG_MASK;

    unsafe {
        core::arch::asm!(
            /*
                r15 = CpuState pointer.
                Keep this until the very end.
            */
            "mov r15, {CPU_DUMP_ADDR}",

            /*
                Build an iretq frame on the target stack.

                We want final RSP to equal CpuState.rsp after iretq.

                Layout required by iretq, same privilege level:

                    rsp + 0x00 = rip
                    rsp + 0x08 = cs
                    rsp + 0x10 = rflags

                Therefore set temporary rsp to target_rsp - 24.
            */
            // "mov rax, rsp",
            "mov rax, {TEST_STACK}",
            "sub rax, 40",

            "mov rcx, [r15 + {OFFSET_RIP}]",
            "mov qword ptr [rax + 0x00], rcx",

            "xor rcx, rcx",
            "mov cx, cs",
            "mov qword ptr [rax + 0x08], rcx",

            "mov rcx, [r15 + {OFFSET_FLAGS}]",
            "mov qword ptr [rax + 0x10], rcx",

            "mov rcx, [r15 + {OFFSET_RSP}]",
            "mov qword ptr [rax + 0x18], rcx",

            "xor rcx, rcx",
            "mov cx, ss",
            "mov qword ptr [rax + 0x20], rcx",

            /*
                Save final iretq-frame pointer in r14.
                r14 will be restored later, right before r15.
            */
            "mov r14, rax",

            /*
                Restore most GPRs.
                Do not restore r14/r15 yet:
                - r15 is still the CpuState pointer
                - r14 holds the final iretq frame pointer
            */
            "mov rax, [r15 + {OFFSET_RAX}]",
            "mov rbx, [r15 + {OFFSET_RBX}]",
            "mov rcx, [r15 + {OFFSET_RCX}]",
            "mov rdx, [r15 + {OFFSET_RDX}]",
            "mov rsi, [r15 + {OFFSET_RSI}]",
            "mov rdi, [r15 + {OFFSET_RDI}]",
            "mov rbp, [r15 + {OFFSET_RBP}]",

            "mov r8,  [r15 + {OFFSET_R8}]",
            "mov r9,  [r15 + {OFFSET_R9}]",
            "mov r10, [r15 + {OFFSET_R10}]",
            "mov r11, [r15 + {OFFSET_R11}]",
            "mov r12, [r15 + {OFFSET_R12}]",
            "mov r13, [r15 + {OFFSET_R13}]",

            /*
                Switch to the artificial iretq frame.
                After this, do not touch memory through rsp except via iretq.
            */
            "mov rsp, r14",

            /*
                Restore r14 and r15 last.
                r15 is still usable as CpuState pointer until the second instruction.
            */
            "mov r14, [r15 + {OFFSET_R14}]",
            "mov r15, [r15 + {OFFSET_R15}]",

            /*
                Restores RIP, CS, RFLAGS, and final RSP.
            */
            "iretq",

            CPU_DUMP_ADDR = in(reg) &test.state,

            // TODO: Restore vector state with XRSTOR after the XSAVE path is validated.

            OFFSET_RIP = const OFFSET_RIP,
            OFFSET_FLAGS = const OFFSET_FLAGS,

            OFFSET_RAX = const OFFSET_RAX,
            OFFSET_RBX = const OFFSET_RBX,
            OFFSET_RCX = const OFFSET_RCX,
            OFFSET_RDX = const OFFSET_RDX,
            OFFSET_RSI = const OFFSET_RSI,
            OFFSET_RDI = const OFFSET_RDI,
            OFFSET_RBP = const OFFSET_RBP,
            OFFSET_RSP = const OFFSET_RSP,

            OFFSET_R8 = const OFFSET_R8,
            OFFSET_R9 = const OFFSET_R9,
            OFFSET_R10 = const OFFSET_R10,
            OFFSET_R11 = const OFFSET_R11,
            OFFSET_R12 = const OFFSET_R12,
            OFFSET_R13 = const OFFSET_R13,
            OFFSET_R14 = const OFFSET_R14,
            OFFSET_R15 = const OFFSET_R15,
            TEST_STACK = const TEST_STACK,

            options(noreturn)
        );
    };
}

pub fn landing_pad(state: &mut CpuState, exception: Option<ExceptionVector>) {
    // Read the memory value
    state.mem0 = unsafe { *(MEM_ADDR as *const u64) };

    let exception = exception.map(|kind| {
        let mut insn = [0u8; 15];
        let test_insn = TEST_INSN.lock();
        let insn_size = core::cmp::min(test_insn.len(), insn.len());

        insn[..insn_size].copy_from_slice(&test_insn[..insn_size]);

        ExceptionInfo {
            kind,
            insn,
            size: insn_size as u8,
        }
    });

    {
        // run some logic
        match DATASET.lock().as_mut() {
            Some(d) => d.after_test(*TEST_ID.lock(), state, exception),
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
    let has_osxsave = f1.has_oxsave();

    let f7 = cpuid.get_extended_feature_info().unwrap();
    let has_avx512f = f7.has_avx512f();

    println!("SSE: {}", has_sse && has_sse2);
    println!("AVX usable: {}", has_avx);
    println!("AVX512ER usable: {}", f7.has_avx512er());
    println!("XSAVE: {}", has_xsave);
    println!("AVX-512F: {}", has_avx512f);
    println!("OSXSAVE: {}", has_osxsave);

    println!("Cr0.TS: {}", Cr0::read().contains(Cr0Flags::TASK_SWITCHED));
    println!(
        "Cr0.EM: {}",
        Cr0::read().contains(Cr0Flags::EMULATE_COPROCESSOR)
    );

    assert!(has_sse && has_sse2 && has_avx && has_xsave && has_avx512f && has_osxsave);
}
