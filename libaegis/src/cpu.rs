use crate::compressible::Compressible;

use core::mem::MaybeUninit;

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

    pub mem0: u64, // A scratch field for testing memory diffs
}

impl CpuState {
    // Creates a null cpu state
    pub const fn zero() -> Self {
        Self {
            mem0: 0,
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
    use super::AvxState;

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
