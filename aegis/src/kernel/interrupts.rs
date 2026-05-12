use crate::{
    kernel::interrupts::hardware::{PICS, keyboard_interrupt_handler, timer_interrupt_handler},
    testing::harness::CPU_DUMP_START,
};
use core::fmt::Display;
use libaegis::cpu::{
    CpuState, OFFSET_CS, OFFSET_DS, OFFSET_ES, OFFSET_FLAGS, OFFSET_FS, OFFSET_GS, OFFSET_R8,
    OFFSET_R9, OFFSET_R10, OFFSET_R11, OFFSET_R12, OFFSET_R13, OFFSET_R14, OFFSET_R15, OFFSET_RAX,
    OFFSET_RBP, OFFSET_RBX, OFFSET_RCX, OFFSET_RDI, OFFSET_RDX, OFFSET_RIP, OFFSET_RSI, OFFSET_RSP,
    OFFSET_SS,
};
use spin::Mutex;
use x86_64::{
    VirtAddr,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame},
};

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

pub type UnifiedHandler =
    fn(exception: ExceptionVector, stack_frame: InterruptStackFrame, error_code: Option<u64>);

pub type ExceptionHandler = fn(
    cpu_state: &CpuState,
    stack_frame: &mut InterruptStackFrame,
    error_code: Option<u64>,
    vector: ExceptionVector,
);

/// The unified handler is a function pointer that can be set by the user to
/// handle exceptions in a custom way.
/// It takes an exception vector, the stack frame at the time of the exception,
/// and an optional error code (some exceptions do not have an error code).
#[unsafe(no_mangle)]
pub static mut UNIFIED_HANDLER: ExceptionHandler = default_handler;

fn default_handler(
    _cpu_state: &CpuState,
    stack_frame: &mut InterruptStackFrame,
    error_code: Option<u64>,
    vector: ExceptionVector,
) {
    panic!(" ERROR({vector})({error_code:?})\n{stack_frame:#?}");
}

#[unsafe(no_mangle)]
pub extern "sysv64" fn handler_trampoline(
    cpu_state: *const CpuState,
    stack_frame: *mut InterruptStackFrame,
    error_code: u64,
    vector: u64,
    has_error_code: u64,
) {
    let exception_vector =
        ExceptionVector::try_from(vector as u8).unwrap_or(ExceptionVector::Unknown);

    let error_code_opt = if has_error_code != 0 {
        Some(error_code)
    } else {
        None
    };

    unsafe {
        UNIFIED_HANDLER(
            &*cpu_state,
            &mut *stack_frame,
            error_code_opt,
            exception_vector,
        );
    }
}

/// Handles the interrupt descriptor table
pub struct InterruptManager;

// macro that sets a handler in the IDT, takes 2 arguments: the idt entry and the handler function
macro_rules! set_handler {
    ($idt:expr, $entry:ident, $handler:ident) => {
        $idt.$entry
            .set_handler_addr(VirtAddr::new(
                error_handler::<false, { ExceptionVector::$handler as u8 }> as *const () as u64,
            ))
            .set_stack_index(gdt::GENERIC_IST_INDEX);
    };
}

macro_rules! set_handler_with_error_code {
    ($idt:expr, $entry:ident, $handler:ident) => {
        $idt.$entry
            .set_handler_addr(VirtAddr::new(
                error_handler::<true, { ExceptionVector::$handler as u8 }> as *const () as u64,
            ))
            .set_stack_index(gdt::GENERIC_IST_INDEX);
    };
}

impl InterruptManager {
    fn init_idt() {
        let mut idt = IDT.lock();

        unsafe {
            set_handler!(idt, divide_error, Division);
            set_handler!(idt, debug, Debug);
            set_handler!(idt, non_maskable_interrupt, NonMaskableInterrupt);
            set_handler!(idt, breakpoint, Breakpoint);
            set_handler!(idt, overflow, Overflow);
            set_handler!(idt, bound_range_exceeded, BoundRange);
            set_handler!(idt, invalid_opcode, InvalidOpcode);
            set_handler!(idt, device_not_available, DeviceNotAvailable);
            set_handler_with_error_code!(idt, invalid_tss, InvalidTss);
            set_handler_with_error_code!(idt, segment_not_present, SegmentNotPresent);
            set_handler_with_error_code!(idt, stack_segment_fault, Stack);
            set_handler_with_error_code!(idt, general_protection_fault, GeneralProtection);
            set_handler_with_error_code!(idt, page_fault, Page);
            set_handler!(idt, x87_floating_point, X87FloatingPoint);
            set_handler_with_error_code!(idt, alignment_check, AlignmentCheck);
            set_handler!(idt, simd_floating_point, SimdFloatingPoint);
            set_handler!(idt, virtualization, Virtualization);
            set_handler_with_error_code!(idt, cp_protection_exception, ControlProtection);
            set_handler!(idt, hv_injection_exception, HypervisorInjection);
            set_handler_with_error_code!(idt, vmm_communication_exception, VmmCommunication);
            set_handler_with_error_code!(idt, security_exception, Security);

            // TODO: MachineCheck;

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

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ExceptionVector {
    /// Error during Division
    Division = 0x00,

    /// Debug
    Debug = 0x01,

    /// Non-Maskable Interrupt
    NonMaskableInterrupt = 0x02,

    /// Breakpoint
    Breakpoint = 0x03,

    /// Overflow
    Overflow = 0x04,

    /// Bound Range Exceeded
    BoundRange = 0x05,

    /// Invalid Opcode
    InvalidOpcode = 0x06,

    /// Device Not Available
    DeviceNotAvailable = 0x07,

    /// Double Fault
    Double = 0x08,

    /// Invalid TSS
    InvalidTss = 0x0A,

    /// Segment Not Present
    SegmentNotPresent = 0x0B,

    /// Stack Fault
    Stack = 0x0C,

    /// General Protection Fault
    GeneralProtection = 0x0D,

    /// Page Fault
    Page = 0x0E,

    /// x87 Floating-Point Exception
    X87FloatingPoint = 0x10,

    /// Alignment Check
    AlignmentCheck = 0x11,

    /// Machine Check
    MachineCheck = 0x12,

    /// SIMD Floating-Point Exception
    SimdFloatingPoint = 0x13,

    /// Virtualization Exception (Intel-only)
    Virtualization = 0x14,

    /// Control Protection Exception
    ControlProtection = 0x15,

    /// Hypervisor Injection (AMD-only)
    HypervisorInjection = 0x1C,

    /// VMM Communication (AMD-only)
    VmmCommunication = 0x1D,

    /// Security Exception
    Security = 0x1E,

    /// Unknown Exception
    Unknown = 0xFF,
}

impl Into<u8> for ExceptionVector {
    fn into(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for ExceptionVector {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(ExceptionVector::Division),
            0x01 => Ok(ExceptionVector::Debug),
            0x02 => Ok(ExceptionVector::NonMaskableInterrupt),
            0x03 => Ok(ExceptionVector::Breakpoint),
            0x04 => Ok(ExceptionVector::Overflow),
            0x05 => Ok(ExceptionVector::BoundRange),
            0x06 => Ok(ExceptionVector::InvalidOpcode),
            0x07 => Ok(ExceptionVector::DeviceNotAvailable),
            0x08 => Ok(ExceptionVector::Double),
            0x0A => Ok(ExceptionVector::InvalidTss),
            0x0B => Ok(ExceptionVector::SegmentNotPresent),
            0x0C => Ok(ExceptionVector::Stack),
            0x0D => Ok(ExceptionVector::GeneralProtection),
            0x0E => Ok(ExceptionVector::Page),
            0x10 => Ok(ExceptionVector::X87FloatingPoint),
            0x11 => Ok(ExceptionVector::AlignmentCheck),
            0x12 => Ok(ExceptionVector::MachineCheck),
            0x13 => Ok(ExceptionVector::SimdFloatingPoint),
            0x14 => Ok(ExceptionVector::Virtualization),
            0x15 => Ok(ExceptionVector::ControlProtection),
            0x1C => Ok(ExceptionVector::HypervisorInjection),
            0x1D => Ok(ExceptionVector::VmmCommunication),
            0x1E => Ok(ExceptionVector::Security),
            _ => Err(()),
        }
    }
}

impl Display for ExceptionVector {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = match self {
            ExceptionVector::Division => "Division Error",
            ExceptionVector::Debug => "Debug",
            ExceptionVector::NonMaskableInterrupt => "Non-Maskable Interrupt",
            ExceptionVector::Breakpoint => "Breakpoint",
            ExceptionVector::Overflow => "Overflow",
            ExceptionVector::BoundRange => "Bound Range Exceeded",
            ExceptionVector::InvalidOpcode => "Invalid Opcode",
            ExceptionVector::DeviceNotAvailable => "Device Not Available",
            ExceptionVector::Double => "Double Fault",
            ExceptionVector::InvalidTss => "Invalid TSS",
            ExceptionVector::SegmentNotPresent => "Segment Not Present",
            ExceptionVector::Stack => "Stack Fault",
            ExceptionVector::GeneralProtection => "General Protection Fault",
            ExceptionVector::Page => "Page Fault",
            ExceptionVector::X87FloatingPoint => "x87 Floating-Point Exception",
            ExceptionVector::AlignmentCheck => "Alignment Check",
            ExceptionVector::MachineCheck => "Machine Check",
            ExceptionVector::SimdFloatingPoint => "SIMD Floating-Point Exception",
            ExceptionVector::Virtualization => "Virtualization Exception",
            ExceptionVector::ControlProtection => "Control Protection Exception",
            ExceptionVector::HypervisorInjection => "Hypervisor Injection (AMD-only)",
            ExceptionVector::VmmCommunication => "VMM Communication (AMD-only)",
            ExceptionVector::Security => "Security Exception",
            ExceptionVector::Unknown => "Unknown Exception",
        };
        write!(f, "{name}")
    }
}

pub extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
}

#[unsafe(naked)]
#[doc(hidden)]
pub extern "C" fn error_handler<const HasErrorCode: bool, const VECTOR: u8>() {
    core::arch::naked_asm!(
        /*
            Error-code exception entry stack in x86-64 long mode:

            rsp + 0x00 = error_code
            rsp + 0x08 = rip
            rsp + 0x10 = cs
            rsp + 0x18 = rflags
            rsp + 0x20 = old_rsp
            rsp + 0x28 = old_ss

            This wrapper is valid for exceptions that push an error code:
            #DF, #TS, #NP, #SS, #GP, #PF, #AC, #CP.
        */
        "cld",

        /*
            For exeptions without an error code, the CPU does not push an error
            code.
            In that case, we synthesize an error code of 0 by pushing it
            ourselves.
        */
        "cmp {HAS_ERROR_CODE}, 0",
        "jne 2f",
            "push 0",
        "2:",


        /*
            Save all GPRs.

            After these pushes:

            rsp + 0x00 = rax
            rsp + 0x08 = rbx
            rsp + 0x10 = rcx
            rsp + 0x18 = rdx
            rsp + 0x20 = rsi
            rsp + 0x28 = rdi
            rsp + 0x30 = rbp
            rsp + 0x38 = r8
            rsp + 0x40 = r9
            rsp + 0x48 = r10
            rsp + 0x50 = r11
            rsp + 0x58 = r12
            rsp + 0x60 = r13
            rsp + 0x68 = r14
            rsp + 0x70 = r15

            rsp + 0x78 = error_code
            rsp + 0x80 = rip
            rsp + 0x88 = cs
            rsp + 0x90 = rflags
            rsp + 0x98 = old_rsp
            rsp + 0xA0 = old_ss
        */

        "push r15",
        "push r14",
        "push r13",
        "push r12",
        "push r11",
        "push r10",
        "push r9",
        "push r8",
        "push rbp",
        "push rdi",
        "push rsi",
        "push rdx",
        "push rcx",
        "push rbx",
        "push rax",



        /*
            rdi = CpuState dump destination.
        */
        "mov rdi, {CPU_DUMP_ADDR}",

        /*
            Save GPRs from the saved stack frame.
        */
        "mov rax, [rsp + 0x00]",
        "mov [rdi + {OFFSET_RAX}], rax",

        "mov rax, [rsp + 0x08]",
        "mov [rdi + {OFFSET_RBX}], rax",

        "mov rax, [rsp + 0x10]",
        "mov [rdi + {OFFSET_RCX}], rax",

        "mov rax, [rsp + 0x18]",
        "mov [rdi + {OFFSET_RDX}], rax",

        "mov rax, [rsp + 0x20]",
        "mov [rdi + {OFFSET_RSI}], rax",

        "mov rax, [rsp + 0x28]",
        "mov [rdi + {OFFSET_RDI}], rax",

        "mov rax, [rsp + 0x30]",
        "mov [rdi + {OFFSET_RBP}], rax",

        "mov rax, [rsp + 0x38]",
        "mov [rdi + {OFFSET_R8}], rax",

        "mov rax, [rsp + 0x40]",
        "mov [rdi + {OFFSET_R9}], rax",

        "mov rax, [rsp + 0x48]",
        "mov [rdi + {OFFSET_R10}], rax",

        "mov rax, [rsp + 0x50]",
        "mov [rdi + {OFFSET_R11}], rax",

        "mov rax, [rsp + 0x58]",
        "mov [rdi + {OFFSET_R12}], rax",

        "mov rax, [rsp + 0x60]",
        "mov [rdi + {OFFSET_R13}], rax",

        "mov rax, [rsp + 0x68]",
        "mov [rdi + {OFFSET_R14}], rax",

        "mov rax, [rsp + 0x70]",
        "mov [rdi + {OFFSET_R15}], rax",

        /*
            Save RIP and interrupted RFLAGS.
        */
        "mov rax, [rsp + 0x80]",
        "mov [rdi + {OFFSET_RIP}], rax",

        "mov rax, [rsp + 0x90]",
        "mov [rdi + {OFFSET_FLAGS}], rax",

        /*
            Save interrupted RSP.

            In 64-bit mode, the CPU interrupt frame contains old RSP.
            Do not synthesize it with LEA for same-CPL exceptions.
        */
        "mov rax, [rsp + 0x98]",
        "mov [rdi + {OFFSET_RSP}], rax",

        /*
            Save segment selectors.

            CS and SS both come from the interrupt frame.
        */
        "mov ax, word ptr [rsp + 0x88]",
        "mov word ptr [rdi + {OFFSET_CS}], ax",

        "mov word ptr [rdi + {OFFSET_DS}], ds",
        "mov word ptr [rdi + {OFFSET_ES}], es",
        "mov word ptr [rdi + {OFFSET_FS}], fs",
        "mov word ptr [rdi + {OFFSET_GS}], gs",

        "mov ax, word ptr [rsp + 0xA0]",
        "mov word ptr [rdi + {OFFSET_SS}], ax",

        /*
            Save extended CPU state.

            Keep this disabled temporarily if you are still debugging
            iretq corruption. A wrong OFFSET_AVX, bad alignment, or
            undersized XSAVE buffer can overwrite unrelated memory.
        */
        // "xor ecx, ecx",
        // "xgetbv",
        // "xsave64 [rdi + {OFFSET_AVX}]",

        /*
            Call inner handler.

            SysV x86-64 ABI args:

            rdi = *const CpuState
            rsi = *const InterruptStackFrame
            rdx = error code
            rcx = vector
            r8  = has_error_code (0 or 1)
        */
        "lea rsi, [rsp + 0x80]",
        "mov rdx, [rsp + 0x78]",
        "mov rcx, {VECTOR}",
        "mov r8, {HAS_ERROR_CODE}",

        /*
            Align stack before calling normal Rust code.

            Before call:
            - rsp must be 16-byte aligned.
            - call then pushes an 8-byte return address.
            - callee enters with rsp % 16 == 8, as SysV expects.
        */
        "mov r11, rsp",
        "and rsp, -16",
        "sub rsp, 16",
        "mov [rsp], r11",

        "mov rax, [rip + {handler_slot}]",
        "call rax",

        /*
            Restore saved-frame rsp.
        */
        "mov rsp, [rsp]",

        /*
            Restore GPRs.
        */
        "pop rax",
        "pop rbx",
        "pop rcx",
        "pop rdx",
        "pop rsi",
        "pop rdi",
        "pop rbp",
        "pop r8",
        "pop r9",
        "pop r10",
        "pop r11",
        "pop r12",
        "pop r13",
        "pop r14",
        "pop r15",

        /*
            Drop CPU-pushed error code.

            After this:
            rsp + 0x00 = rip
            rsp + 0x08 = cs
            rsp + 0x10 = rflags
            rsp + 0x18 = old_rsp
            rsp + 0x20 = old_ss

            iretq consumes that frame.
        */
        "add rsp, 8",

        "iretq",

        CPU_DUMP_ADDR = const CPU_DUMP_START,

        // OFFSET_AVX = const OFFSET_AVX,

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

        OFFSET_CS = const OFFSET_CS,
        OFFSET_DS = const OFFSET_DS,
        OFFSET_ES = const OFFSET_ES,
        OFFSET_FS = const OFFSET_FS,
        OFFSET_GS = const OFFSET_GS,
        OFFSET_SS = const OFFSET_SS,
        VECTOR = const VECTOR,
        HAS_ERROR_CODE = const HasErrorCode as u8,

        handler_slot = sym UNIFIED_HANDLER,
        options()
    );
}
