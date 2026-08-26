use crate::compressible::Compressible;

use core::mem::MaybeUninit;

/// Bytes transported at `MEM0_ADDR` by the test harness. This covers the full
/// legacy FXSAVE image and reaches the independently-addressed `mem1` word.
pub const SCRATCH_MEMORY_SIZE: usize = 512;

/// A representation of the x86-64 cpu state
#[cfg_attr(not(feature = "std"), repr(align(64)))] // XSAVE requires 64-byte alignment
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuState {
    pub avx: AvxState,
    /// Canonical physical x87/MMX register file, transported separately from
    /// the backing FXSAVE/XSAVE image.
    pub fpu: FpuState,
    pub rip: u64,
    pub seg: SegState,
    pub gpr: GPRState,
    pub flags: FlagState,

    /// Bounded raw scratch memory at `MEM0_ADDR`, used by environment-memory
    /// instructions. `mem0` and `mem1` remain legacy word views at offsets 0
    /// and 0x100 respectively.
    pub scratch_memory: [u8; SCRATCH_MEMORY_SIZE],
    /// Zero means the raw scratch transport was not requested; otherwise this
    /// is `SCRATCH_MEMORY_SIZE`.
    pub scratch_memory_len: u16,
    /// Two independently-addressed legacy scratch-word views. `mem0` is
    /// conventionally the source, `mem1` the destination.
    pub mem0: u64,
    pub mem1: u64,
}

impl CpuState {
    // Creates a null cpu state
    pub const fn zero() -> Self {
        Self {
            scratch_memory: [0; SCRATCH_MEMORY_SIZE],
            scratch_memory_len: 0,
            mem0: 0,
            mem1: 0,
            avx: AvxState { data: [0; 4096] },
            fpu: FpuState::zero(),
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

impl Default for CpuState {
    fn default() -> Self {
        Self::zero()
    }
}

/// The flags in the x86-64 cpu
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlagState(pub u64);

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
    const XSTATE_BV_SSE: u64 = 1 << 1;
    const XSTATE_BV_AVX: u64 = 1 << 2;
    const XSTATE_BV_ZMM_HI256: u64 = 1 << 6;
    const XSTATE_BV_HI16_ZMM: u64 = 1 << 7;

    fn xstate_bv(&self) -> u64 {
        u64::from_le_bytes(
            self.data[Self::XSTATE_BV_OFFSET..Self::XSTATE_BV_OFFSET + 8]
                .try_into()
                .unwrap(),
        )
    }

    fn set_xstate_bv(&mut self, xstate_bv: u64) {
        self.data[Self::XSTATE_BV_OFFSET..Self::XSTATE_BV_OFFSET + 8]
            .copy_from_slice(&xstate_bv.to_le_bytes());
    }

    fn mark_xstate_components(&mut self, components: u64) {
        self.set_xstate_bv(self.xstate_bv() | components);
    }

    // Legacy x87/MMX portion of the FXSAVE image. The x87 registers occupy
    // 16-byte slots in *logical* ST(0)..ST(7) order; only the low 64-bit
    // significand is the MMX value. The abridged tag byte is physical order.
    const FXSAVE_X87_CONTROL_OFFSET: usize = 0;
    const FXSAVE_X87_STATUS_OFFSET: usize = 2;
    const FXSAVE_X87_TAG_OFFSET: usize = 4;
    const FXSAVE_X87_OPCODE_OFFSET: usize = 6;
    const FXSAVE_X87_IP_OFFSET: usize = 8;
    const FXSAVE_X87_DP_OFFSET: usize = 16;
    const FXSAVE_MXCSR_OFFSET: usize = 24;
    const FXSAVE_ST_OFFSET: usize = 32;
    const FXSAVE_ST_STRIDE: usize = 16;
    const FXSAVE_ST_VALUE_SIZE: usize = 10;
    const MXCSR_DEFAULT: u32 = 0x1f80;

    /// Prepares the legacy FXSAVE portion for `FXRSTOR` from the canonical
    /// physical x87/MMX register file. The SSE/AVX portions remain untouched.
    pub fn prepare_fpu_restore(&mut self, fpu: &FpuState) {
        self.data[..Self::XMM_OFFSET].fill(0);
        self.data[Self::FXSAVE_X87_CONTROL_OFFSET..Self::FXSAVE_X87_CONTROL_OFFSET + 2]
            .copy_from_slice(&fpu.control.to_le_bytes());
        self.data[Self::FXSAVE_X87_STATUS_OFFSET..Self::FXSAVE_X87_STATUS_OFFSET + 2]
            .copy_from_slice(&fpu.status.to_le_bytes());
        self.data[Self::FXSAVE_X87_TAG_OFFSET] = fpu.tag;
        self.data[Self::FXSAVE_X87_OPCODE_OFFSET..Self::FXSAVE_X87_OPCODE_OFFSET + 2]
            .copy_from_slice(&fpu.opcode.to_le_bytes());
        self.data[Self::FXSAVE_X87_IP_OFFSET..Self::FXSAVE_X87_IP_OFFSET + 8]
            .copy_from_slice(&fpu.instruction_pointer.to_le_bytes());
        self.data[Self::FXSAVE_X87_DP_OFFSET..Self::FXSAVE_X87_DP_OFFSET + 8]
            .copy_from_slice(&fpu.data_pointer.to_le_bytes());
        self.data[Self::FXSAVE_MXCSR_OFFSET..Self::FXSAVE_MXCSR_OFFSET + 4]
            .copy_from_slice(&Self::MXCSR_DEFAULT.to_le_bytes());
        for logical_index in 0..8 {
            let offset = Self::FXSAVE_ST_OFFSET + logical_index * Self::FXSAVE_ST_STRIDE;
            self.data[offset..offset + Self::FXSAVE_ST_VALUE_SIZE]
                .copy_from_slice(fpu.st(logical_index));
        }
    }

    /// Extracts the canonical physical x87/MMX register file from FXSAVE.
    pub fn fpu_state(&self) -> FpuState {
        let mut fpu = FpuState {
            control: u16::from_le_bytes(self.data[0..2].try_into().unwrap()),
            status: u16::from_le_bytes(self.data[2..4].try_into().unwrap()),
            tag: self.data[Self::FXSAVE_X87_TAG_OFFSET],
            opcode: u16::from_le_bytes(self.data[6..8].try_into().unwrap()),
            instruction_pointer: u64::from_le_bytes(self.data[8..16].try_into().unwrap()),
            data_pointer: u64::from_le_bytes(self.data[16..24].try_into().unwrap()),
            registers: [[0; 10]; 8],
        };
        for logical_index in 0..8 {
            let offset = Self::FXSAVE_ST_OFFSET + logical_index * Self::FXSAVE_ST_STRIDE;
            let mut value = [0; 10];
            value.copy_from_slice(&self.data[offset..offset + Self::FXSAVE_ST_VALUE_SIZE]);
            fpu.set_st(logical_index, value);
        }
        fpu
    }

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
        self.data[Self::XMM_OFFSET + idx * 16..][..16].copy_from_slice(val);
        self.mark_xstate_components(Self::XSTATE_BV_SSE);
    }

    /// Returns the full 256-bit value of YMM register `idx` (0-15) as 32 bytes (little-endian).
    pub fn get_ymm(&self, idx: usize) -> [u8; 32] {
        assert!(idx < 16);
        let mut out = [0u8; 32];
        out[..16].copy_from_slice(&self.data[Self::XMM_OFFSET + idx * 16..][..16]);
        out[16..32].copy_from_slice(&self.data[Self::YMM_HI_OFFSET + idx * 16..][..16]);
        out
    }

    /// Writes 32 bytes into YMM register `idx` (0-15) and marks SSE and AVX
    /// components in XSTATE_BV so XRSTOR restores both halves.
    pub fn set_ymm(&mut self, idx: usize, val: &[u8; 32]) {
        assert!(idx < 16);
        self.data[Self::XMM_OFFSET + idx * 16..][..16].copy_from_slice(&val[..16]);
        self.data[Self::YMM_HI_OFFSET + idx * 16..][..16].copy_from_slice(&val[16..32]);
        self.mark_xstate_components(Self::XSTATE_BV_SSE | Self::XSTATE_BV_AVX);
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
        if idx < 16 {
            self.data[Self::XMM_OFFSET + idx * 16..][..16].copy_from_slice(&val[..16]);
            self.data[Self::YMM_HI_OFFSET + idx * 16..][..16].copy_from_slice(&val[16..32]);
            self.data[Self::ZMM_HI256_OFFSET + idx * 32..][..32].copy_from_slice(&val[32..64]);
            self.mark_xstate_components(
                Self::XSTATE_BV_SSE | Self::XSTATE_BV_AVX | Self::XSTATE_BV_ZMM_HI256,
            );
        } else {
            self.data[Self::HI16_ZMM_OFFSET + (idx - 16) * 64..][..64].copy_from_slice(val);
            self.mark_xstate_components(Self::XSTATE_BV_HI16_ZMM);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AvxState, FpuState};

    #[test]
    fn set_xmm_marks_sse_component_in_xstate_bv() {
        let mut avx = AvxState::default();
        avx.set_xstate_bv(1 << 0);

        avx.set_xmm(3, &[0xAA; 16]);

        assert_eq!(avx.xstate_bv(), (1 << 0) | AvxState::XSTATE_BV_SSE);
    }

    #[test]
    fn set_ymm_marks_sse_and_avx_components_in_xstate_bv() {
        let mut avx = AvxState::default();

        avx.set_ymm(7, &[0xBB; 32]);

        assert_eq!(
            avx.xstate_bv(),
            AvxState::XSTATE_BV_SSE | AvxState::XSTATE_BV_AVX
        );
    }

    #[test]
    fn set_zmm_low_marks_sse_avx_and_zmm_hi256_components() {
        let mut avx = AvxState::default();

        avx.set_zmm(2, &[0xCC; 64]);

        assert_eq!(
            avx.xstate_bv(),
            AvxState::XSTATE_BV_SSE | AvxState::XSTATE_BV_AVX | AvxState::XSTATE_BV_ZMM_HI256
        );
    }

    #[test]
    fn set_zmm_high_marks_hi16_zmm_component() {
        let mut avx = AvxState::default();
        avx.set_xstate_bv(1 << 0);

        avx.set_zmm(20, &[0xDD; 64]);

        assert_eq!(avx.xstate_bv(), (1 << 0) | AvxState::XSTATE_BV_HI16_ZMM);
    }

    #[test]
    fn fpu_fxsave_image_round_trips_without_clobbering_xmm() {
        let mut avx = AvxState::default();
        avx.set_xmm(1, &[0x5a; 16]);
        let mut fpu = FpuState::zero();
        fpu.control = 0x027f;
        fpu.status = 0x3800;
        fpu.tag = 0x81;
        fpu.opcode = 0x123;
        fpu.instruction_pointer = 0x1234_5678_9abc_def0;
        fpu.data_pointer = 0x0fed_cba9_8765_4321;
        fpu.registers[0] = [0x11; 10];
        fpu.registers[1] = [0x22; 10];
        fpu.registers[7] = [0x88; 10];

        avx.prepare_fpu_restore(&fpu);

        // FXSAVE payload slots are logical: with TOP=7, logical ST0 is
        // canonical physical R7, while logical ST1 is R0.
        let st0 = AvxState::FXSAVE_ST_OFFSET;
        let st1 = st0 + AvxState::FXSAVE_ST_STRIDE;
        assert_eq!(&avx.data[st0..st0 + 10], &[0x88; 10]);
        assert_eq!(&avx.data[st1..st1 + 10], &[0x11; 10]);
        assert_eq!(avx.fpu_state(), fpu);
        assert_eq!(avx.get_xmm(1), [0x5a; 16]);
    }

    #[test]
    fn fpu_mmx_and_x87_views_alias_physical_registers() {
        let mut fpu = FpuState::zero();
        fpu.initialize_mmx();
        // TOP=3 means logical ST0 is physical FPU slot 3, the same slot MM3
        // exposes through its low 64-bit view.
        fpu.status = 3 << 11;
        fpu.set_st(0, [0x11; 10]);

        assert_eq!(fpu.mmx(3), 0x1111_1111_1111_1111);
        fpu.set_mmx(3, 0x1122_3344_5566_7788);
        assert_eq!(&fpu.st(0)[..8], &0x1122_3344_5566_7788u64.to_le_bytes());
        assert_eq!(fpu.registers[3][8..], [0x11; 2]);
        assert_eq!(fpu.tag, 0xff);
    }
}

/// Canonical physical x87/MMX register file.
///
/// `registers` is in canonical physical-slot order. FXSAVE stores payload
/// slots in logical ST(0)..ST(7) order, so conversion occurs at its boundary.
/// x87 logical `ST(i)` and MMX are views over this one state: TOP in `status`
/// maps x87 logical slots, while `MM(i)` is the low 64 bits of physical slot
/// `i`. `tag` is FXSAVE's abridged tag byte in physical-slot order.
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FpuState {
    pub control: u16,
    pub status: u16,
    pub tag: u8,
    pub opcode: u16,
    pub instruction_pointer: u64,
    pub data_pointer: u64,
    pub registers: [[u8; 10]; 8],
}

impl FpuState {
    pub const fn zero() -> Self {
        Self {
            control: 0x037f,
            status: 0,
            tag: 0,
            opcode: 0,
            instruction_pointer: 0,
            data_pointer: 0,
            registers: [[0; 10]; 8],
        }
    }

    pub const fn top(&self) -> usize {
        ((self.status >> 11) & 7) as usize
    }

    pub const fn physical_for_st(&self, logical_index: usize) -> usize {
        (self.top() + logical_index) & 7
    }

    pub fn st(&self, logical_index: usize) -> &[u8; 10] {
        &self.registers[self.physical_for_st(logical_index)]
    }

    pub fn set_st(&mut self, logical_index: usize, value: [u8; 10]) {
        let physical_index = self.physical_for_st(logical_index);
        self.registers[physical_index] = value;
    }

    pub fn logical_tag(&self) -> u8 {
        let mut tag = 0;
        for logical_index in 0..8 {
            if self.tag & (1 << self.physical_for_st(logical_index)) != 0 {
                tag |= 1 << logical_index;
            }
        }
        tag
    }

    pub fn set_logical_tag(&mut self, logical_tag: u8) {
        self.tag = 0;
        for logical_index in 0..8 {
            if logical_tag & (1 << logical_index) != 0 {
                self.tag |= 1 << self.physical_for_st(logical_index);
            }
        }
    }

    pub fn mmx(&self, physical_index: usize) -> u64 {
        u64::from_le_bytes(self.registers[physical_index][..8].try_into().unwrap())
    }

    pub fn set_mmx(&mut self, physical_index: usize, value: u64) {
        self.registers[physical_index][..8].copy_from_slice(&value.to_le_bytes());
    }

    /// Initializes the canonical file to the architectural state used by MMX:
    /// every x87 slot is non-empty and has an all-ones exponent.
    pub fn initialize_mmx(&mut self) {
        self.control = 0x037f;
        self.status = 0;
        self.tag = 0xff;
        for register in &mut self.registers {
            register.fill(0);
            register[8..].copy_from_slice(&u16::MAX.to_le_bytes());
        }
    }
}

impl Default for FpuState {
    fn default() -> Self {
        Self::zero()
    }
}

pub const OFFSET_AVX: usize = core::mem::offset_of!(CpuState, avx);
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
