use crate::compressible::Compressible;

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

#[cfg(not(feature = "std"))]
use alloc::string::String;

use core::{fmt, mem::MaybeUninit};

/// A representation of the x86-64 cpu state
#[cfg_attr(not(feature = "std"), repr(align(64)))] // XSAVE requires 64-byte alignment
#[repr(C)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CpuState {
    pub avx: AvxState,
    pub mmx: MMXState,
    pub rip: u64,
    pub seg: SegState,
    pub gpr: GPRState,
    pub flags: FlagState,
}

impl CpuState {
    // Creates a null cpu state
    pub const fn zero() -> Self {
        Self {
            avx: AvxState { data: [0; 4096] },
            mmx: MMXState {
                mm0: 0,
                mm1: 0,
                mm2: 0,
                mm3: 0,
                mm4: 0,
                mm5: 0,
                mm6: 0,
                mm7: 0,
            },
            rip: 0,
            seg: SegState {
                cs: 0,
                ds: 0,
                es: 0,
                fs: 0,
                gs: 0,
                ss: 0,
            },
            gpr: GPRState {
                rax: 0,
                rbx: 0,
                rcx: 0,
                rdx: 0,
                rsi: 0,
                rdi: 0,
                rbp: 0,
                rsp: 0,
                r8: 0,
                r9: 0,
                r10: 0,
                r11: 0,
                r12: 0,
                r13: 0,
                r14: 0,
                r15: 0,
            },
            flags: FlagState(0),
        }
    }

    pub fn diff(&self, other: &Self) -> Self {
        const CHUNKS: usize = core::mem::size_of::<CpuState>() / 8;

        let buff_1 =
            unsafe { core::slice::from_raw_parts(self as *const Self as *const u64, CHUNKS) };

        let buff_2 =
            unsafe { core::slice::from_raw_parts(other as *const Self as *const u64, CHUNKS) };

        let mut out: MaybeUninit<Self> = MaybeUninit::uninit();

        let buff_3 = unsafe {
            core::slice::from_raw_parts_mut(&mut out as *mut MaybeUninit<Self> as *mut u64, CHUNKS)
        };

        for i in 0..CHUNKS {
            buff_3[i] = buff_1[i] ^ buff_2[i];
        }

        unsafe { out.assume_init() }
    }
}

unsafe impl Compressible for CpuState {}

/// The flags in the x86-64 cpu
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlagState(pub u64);

pub const RFLAGS: &[(u8, &str)] = &[
    (0, "CF"),    // Carry
    (2, "PF"),    // Parity
    (4, "AF"),    // Auxiliary carry
    (6, "ZF"),    // Zero
    (7, "SF"),    // Sign
    (8, "TF"),    // Trap
    (9, "IF"),    // Interrupt enable
    (10, "DF"),   // Direction
    (11, "OF"),   // Overflow
    (12, "IOPL"), // I/O privilege level (2 bits: 12–13)
    (14, "NT"),   // Nested task
    (16, "RF"),   // Resume
    (17, "VM"),   // Virtual 8086
    (18, "AC"),   // Alignment check
    (19, "VIF"),  // Virtual interrupt
    (20, "VIP"),  // Virtual interrupt pending
    (21, "ID"),   // CPUID enable
];

impl fmt::Display for FlagState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (bit, name) in RFLAGS {
            let value = (self.0 >> bit) & 1;
            write!(f, "{}: {} ", name, value)?;
        }

        Ok(())
    }
}

impl FlagState {
    pub fn diff_flags(a: FlagState, b: FlagState) -> String {
        let mut diffs = Vec::new();

        for &(bit, name) in RFLAGS {
            // TODO:
            if name == "OF" {
                continue;
            }

            let mask = if name == "IOPL" {
                0b11u64 << bit
            } else {
                1u64 << bit
            };

            if (a.0 ^ b.0) & mask != 0 {
                diffs.push(name);
            }
        }

        diffs.join(", ")
    }
}

/// The general purpose registers in the x86-64 cpu
#[repr(C)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GPRState {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

/// The segment registers in the x86-64 cpu
#[repr(C)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SegState {
    pub cs: u64,
    pub ds: u64,
    pub es: u64,
    pub fs: u64,
    pub gs: u64,
    pub ss: u64,
}

/// AVX registers in the x86-64 cpu
#[cfg_attr(not(feature = "std"), repr(align(64)))] // XSAVE requires 64-byte alignment
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvxState {
    pub data: [u8; 4096],
}

impl Default for AvxState {
    fn default() -> Self {
        Self { data: [0; 4096] }
    }
}

impl AvxState {
    // XSAVE area offsets (standard Intel layout)
    const XMM_OFFSET: usize = 160; // legacy SSE: XMM part of zmm0-15 (16 bytes each)
    const YMM_HI_OFFSET: usize = 576; // component 2: bits 128-255 of zmm0-15 (16 bytes each)
    const ZMM_HI256_OFFSET: usize = 1152; // component 6: bits 256-511 of zmm0-15 (32 bytes each)
    const HI16_ZMM_OFFSET: usize = 1664; // component 7: full zmm16-31 (64 bytes each)
    const XSTATE_BV_OFFSET: usize = 512; // XSAVE header: XSTATE_BV field (u64)

    /// Returns the full 128-bit value of XMM register `idx` (0-15).
    pub fn get_xmm(&self, idx: usize) -> [u8; 16] {
        assert!(idx < 16);
        let mut out = [0u8; 16];
        out.copy_from_slice(&self.data[Self::XMM_OFFSET + idx * 16..][..16]);
        out
    }

    /// Writes 128 bits into XMM register `idx` (0-15) and marks the SSE
    /// component in XSTATE_BV so XRSTOR restores the XMM state.
    pub fn set_xmm(&mut self, idx: usize, val: &[u8; 16]) {
        assert!(idx < 16);
        let mut xstate_bv = u64::from_le_bytes(
            self.data[Self::XSTATE_BV_OFFSET..Self::XSTATE_BV_OFFSET + 8]
                .try_into()
                .unwrap(),
        );

        self.data[Self::XMM_OFFSET + idx * 16..][..16].copy_from_slice(val);
        xstate_bv |= 1 << 1; // SSE/XMM

        self.data[Self::XSTATE_BV_OFFSET..Self::XSTATE_BV_OFFSET + 8]
            .copy_from_slice(&xstate_bv.to_le_bytes());
    }

    /// Returns the full 512-bit value of ZMM register `idx` (0-31) as 64 bytes (little-endian).
    pub fn get_zmm(&self, idx: usize) -> [u8; 64] {
        assert!(idx < 32);
        let mut out = [0u8; 64];
        if idx < 16 {
            out[..16].copy_from_slice(&self.data[Self::XMM_OFFSET + idx * 16..][..16]);
            out[16..32].copy_from_slice(&self.data[Self::YMM_HI_OFFSET + idx * 16..][..16]);
            out[32..64].copy_from_slice(&self.data[Self::ZMM_HI256_OFFSET + idx * 32..][..32]);
        } else {
            out.copy_from_slice(&self.data[Self::HI16_ZMM_OFFSET + (idx - 16) * 64..][..64]);
        }
        out
    }

    /// Writes 64 bytes into ZMM register `idx` (0-31) and marks the relevant XSAVE
    /// components as valid in XSTATE_BV so XRSTOR will restore them.
    pub fn set_zmm(&mut self, idx: usize, val: &[u8; 64]) {
        assert!(idx < 32);
        let mut xstate_bv = u64::from_le_bytes(
            self.data[Self::XSTATE_BV_OFFSET..Self::XSTATE_BV_OFFSET + 8]
                .try_into()
                .unwrap(),
        );
        if idx < 16 {
            self.data[Self::XMM_OFFSET + idx * 16..][..16].copy_from_slice(&val[..16]);
            self.data[Self::YMM_HI_OFFSET + idx * 16..][..16].copy_from_slice(&val[16..32]);
            self.data[Self::ZMM_HI256_OFFSET + idx * 32..][..32].copy_from_slice(&val[32..64]);
            xstate_bv |= (1 << 1) | (1 << 2) | (1 << 6); // SSE | AVX | ZMM_HI256
        } else {
            self.data[Self::HI16_ZMM_OFFSET + (idx - 16) * 64..][..64].copy_from_slice(val);
            xstate_bv |= (1 << 1) | (1 << 7); // SSE | HI16_ZMM
        }
        self.data[Self::XSTATE_BV_OFFSET..Self::XSTATE_BV_OFFSET + 8]
            .copy_from_slice(&xstate_bv.to_le_bytes());
    }
}

/// MMX registers in the x86-64 cpu
#[repr(C)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MMXState {
    pub mm0: u64,
    pub mm1: u64,
    pub mm2: u64,
    pub mm3: u64,
    pub mm4: u64,
    pub mm5: u64,
    pub mm6: u64,
    pub mm7: u64,
}

pub const OFFSET_AVX: usize =
    core::mem::offset_of!(CpuState, avx) + core::mem::offset_of!(AvxState, data);
pub const OFFSET_MMX: usize = core::mem::offset_of!(CpuState, mmx);
pub const OFFSET_RIP: usize = core::mem::offset_of!(CpuState, rip);
pub const OFFSET_SEG: usize = core::mem::offset_of!(CpuState, seg);
pub const OFFSET_GPR: usize = core::mem::offset_of!(CpuState, gpr);
pub const OFFSET_FLAGS: usize = core::mem::offset_of!(CpuState, flags);

pub const OFFSET_RAX: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, rax);
pub const OFFSET_RBX: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, rbx);
pub const OFFSET_RCX: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, rcx);
pub const OFFSET_RDX: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, rdx);
pub const OFFSET_RSI: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, rsi);
pub const OFFSET_RDI: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, rdi);
pub const OFFSET_RBP: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, rbp);
pub const OFFSET_RSP: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, rsp);
pub const OFFSET_R8: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, r8);
pub const OFFSET_R9: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, r9);
pub const OFFSET_R10: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, r10);
pub const OFFSET_R11: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, r11);
pub const OFFSET_R12: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, r12);
pub const OFFSET_R13: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, r13);
pub const OFFSET_R14: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, r14);
pub const OFFSET_R15: usize = OFFSET_GPR + core::mem::offset_of!(GPRState, r15);

pub const OFFSET_CS: usize = OFFSET_SEG + core::mem::offset_of!(SegState, cs);
pub const OFFSET_DS: usize = OFFSET_SEG + core::mem::offset_of!(SegState, ds);
pub const OFFSET_ES: usize = OFFSET_SEG + core::mem::offset_of!(SegState, es);
pub const OFFSET_FS: usize = OFFSET_SEG + core::mem::offset_of!(SegState, fs);
pub const OFFSET_GS: usize = OFFSET_SEG + core::mem::offset_of!(SegState, gs);
pub const OFFSET_SS: usize = OFFSET_SEG + core::mem::offset_of!(SegState, ss);

pub const OFFSET_MM0: usize = OFFSET_MMX + core::mem::offset_of!(MMXState, mm0);
pub const OFFSET_MM1: usize = OFFSET_MMX + core::mem::offset_of!(MMXState, mm1);
pub const OFFSET_MM2: usize = OFFSET_MMX + core::mem::offset_of!(MMXState, mm2);
pub const OFFSET_MM3: usize = OFFSET_MMX + core::mem::offset_of!(MMXState, mm3);
pub const OFFSET_MM4: usize = OFFSET_MMX + core::mem::offset_of!(MMXState, mm4);
pub const OFFSET_MM5: usize = OFFSET_MMX + core::mem::offset_of!(MMXState, mm5);
pub const OFFSET_MM6: usize = OFFSET_MMX + core::mem::offset_of!(MMXState, mm6);
pub const OFFSET_MM7: usize = OFFSET_MMX + core::mem::offset_of!(MMXState, mm7);

/// Wraps an interrupt handler to save all registers before calling the inner function
#[macro_export]
macro_rules! wrap {
    ($name: ident, $handler: ident) => {
        paste! {

            #[unsafe(no_mangle)]
            #[doc(hidden)]
            pub extern "C" fn [<_ inner _ $name>](state: *const CpuState,
                frame: *const InterruptStackFrame,
                err: PageFaultErrorCode,
                rip: *mut u64) {
                    unsafe {
                        const _: fn(&CpuState, &InterruptStackFrame, PageFaultErrorCode, &mut u64) -> () = $handler;
                        $handler(&*state, &*frame, err, &mut *rip)
                    }
                }


        #[unsafe(naked)]
        #[unsafe(no_mangle)]
        #[doc(hidden)]
        pub extern "C" fn $name() {
            core::arch::naked_asm!(
                // Save the flags
                "pushfq",

                // Clears the direction flag
                "cld",

                // Save rbp
                "push rbp",

                // Save rdi
                "push rdi",

                // Save the original rsp
                "lea rbp, [rsp + 24]",

                // Move our CPUState pointer
                "mov rdi, {CPU_DUMP_ADDR}",

                // Save general purpose registers
                "mov [rdi + {OFFSET_RAX}], rax",
                "mov [rdi + {OFFSET_RBX}], rbx",
                "mov [rdi + {OFFSET_RCX}], rcx",
                "mov [rdi + {OFFSET_RDX}], rdx",
                "mov [rdi + {OFFSET_RSI}], rsi",

                // RDI, RSP, RBP
                "mov [rdi + {OFFSET_R8}],  r8",
                "mov [rdi + {OFFSET_R9}],  r9",
                "mov [rdi + {OFFSET_R10}], r10",
                "mov [rdi + {OFFSET_R11}], r11",
                "mov [rdi + {OFFSET_R12}], r12",
                "mov [rdi + {OFFSET_R13}], r13",
                "mov [rdi + {OFFSET_R14}], r14",
                "mov [rdi + {OFFSET_R15}], r15",

                // Save the segment registers
                "mov [rdi + {OFFSET_CS}], cs",
                "mov [rdi + {OFFSET_DS}], ds",
                "mov [rdi + {OFFSET_ES}], es",
                "mov [rdi + {OFFSET_FS}], fs",
                "mov [rdi + {OFFSET_GS}], gs",
                "mov [rdi + {OFFSET_SS}], ss",

                // Save the mmx registers
                "movq [rdi + {OFFSET_MM0}], mm0",
                "movq [rdi + {OFFSET_MM1}], mm1",
                "movq [rdi + {OFFSET_MM2}], mm2",
                "movq [rdi + {OFFSET_MM3}], mm3",
                "movq [rdi + {OFFSET_MM4}], mm4",
                "movq [rdi + {OFFSET_MM5}], mm5",
                "movq [rdi + {OFFSET_MM6}], mm6",
                "movq [rdi + {OFFSET_MM7}], mm7",

                // Save the avx registers
                "mov eax, 0xFFFFFFFF",
                "mov edx, 0xFFFFFFFF",
                "xsave [rdi + {OFFSET_AVX}]",

                // Save the variables that were "lost"
                "lea rax, [rbp + 56]", // (5 extra values in the interrupt stack frame + error code + 1 mistery value)
                "mov [rdi + {OFFSET_RSP}], rax",
                "mov rax, [rbp - 8]",
                "mov [rdi + {OFFSET_FLAGS}], rax",
                "mov rax, [rbp - 16]",
                "mov [rdi + {OFFSET_RBP}], rax",
                "mov rax, [rbp - 24]",
                "mov [rdi + {OFFSET_RDI}], rax",

                 // Pointer to CPU stack frame
                 "lea rsi, [rbp + 8]",
                 // ERROR CODE
                 "mov rdx, [rbp]",
                 // return address
                 "lea rcx, [rbp + 8]",
                 "mov rax, [rcx]",
                 "mov [rdi + {OFFSET_RIP}], rax",

                 // Call our wrapped function
                 "call {handler}",

                 // Restore the stack
                 "mov rsp, rbp",
                 // Pop the error code
                 "pop rax",
                 "iretq",
                 CPU_DUMP_ADDR = const CPU_DUMP_START,
                 OFFSET_AVX = const  OFFSET_AVX,
                 OFFSET_RIP = const  OFFSET_RIP,
                 OFFSET_FLAGS = const  OFFSET_FLAGS,
                 OFFSET_RAX = const  OFFSET_RAX,
                 OFFSET_RBX = const  OFFSET_RBX,
                 OFFSET_RCX = const  OFFSET_RCX,
                 OFFSET_RDX = const  OFFSET_RDX,
                 OFFSET_RSI = const  OFFSET_RSI,
                 OFFSET_RDI = const  OFFSET_RDI,
                 OFFSET_RBP = const  OFFSET_RBP,
                 OFFSET_RSP = const  OFFSET_RSP,
                 OFFSET_R8 = const  OFFSET_R8,
                 OFFSET_R9 = const  OFFSET_R9,
                 OFFSET_R10 = const  OFFSET_R10,
                 OFFSET_R11 = const  OFFSET_R11,
                 OFFSET_R12 = const  OFFSET_R12,
                 OFFSET_R13 = const  OFFSET_R13,
                 OFFSET_R14 = const  OFFSET_R14,
                 OFFSET_R15 = const  OFFSET_R15,
                 OFFSET_CS = const  OFFSET_CS,
                 OFFSET_DS = const  OFFSET_DS,
                 OFFSET_ES = const  OFFSET_ES,
                 OFFSET_FS = const  OFFSET_FS,
                 OFFSET_GS = const  OFFSET_GS,
                 OFFSET_SS = const  OFFSET_SS,
                 OFFSET_MM0 = const  OFFSET_MM0,
                 OFFSET_MM1 = const  OFFSET_MM1,
                 OFFSET_MM2 = const  OFFSET_MM2,
                 OFFSET_MM3 = const  OFFSET_MM3,
                 OFFSET_MM4 = const  OFFSET_MM4,
                 OFFSET_MM5 = const  OFFSET_MM5,
                 OFFSET_MM6 = const  OFFSET_MM6,
                 OFFSET_MM7 = const  OFFSET_MM7,
                 handler = sym [<_ inner _ $name>],
                options()
            );
        }}
    };
}
