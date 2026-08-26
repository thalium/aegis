#[cfg(not(feature = "std"))]
extern crate core;

#[cfg(not(feature = "std"))]
use core::mem::MaybeUninit;
#[cfg(feature = "std")]
use std::mem::MaybeUninit;

#[cfg(not(feature = "std"))]
use core::fmt::Display;

#[cfg(feature = "std")]
use std::fmt::Display;

use crate::{
    compressible::{CodecError, Compressible},
    cpu::CpuState,
};

pub type TestId = usize;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
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

impl From<ExceptionVector> for u8 {
    fn from(val: ExceptionVector) -> Self {
        val as u8
    }
}

impl TryFrom<u8> for ExceptionVector {
    type Error = CodecError;

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
            _ => Err(CodecError),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionInfo {
    pub kind: ExceptionVector,
    pub insn: [u8; 15],
    pub size: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // Avoid heap allocation in the kernel test loop.
pub enum TestOutcome {
    Completed(CpuState),
    Exception(ExceptionInfo),
}

/// A test to run
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestCase {
    /// The id of this test
    pub id: TestId,

    /// The initial state
    pub state: CpuState,

    /// The x86 instruction to run
    pub insn: [u8; 15],

    /// The number of bytes in the instruction
    pub size: u8,
}

impl TestCase {
    /// Writes a TestCase as a sequence of bytes to an address
    pub fn to_bytes<'a>(&self, mut buff: &'a mut [u8]) -> Result<&'a mut [u8], CodecError> {
        if buff.len() < 8 {
            return Err(CodecError);
        }

        buff[..8].copy_from_slice(&self.id.to_le_bytes());
        buff = &mut buff[8..];

        buff = self.state.to_bytes(buff)?;

        if buff.len() < 16 {
            return Err(CodecError);
        }

        buff[..15].copy_from_slice(&self.insn);
        buff = &mut buff[15..];

        if buff.is_empty() {
            return Err(CodecError);
        }

        buff[0] = self.size;
        buff = &mut buff[1..];

        Ok(buff)
    }

    pub fn from_bytes(mut buff: &[u8]) -> Result<(&[u8], Self), CodecError> {
        if buff.len() < 8 {
            return Err(CodecError);
        }

        let id = usize::from_le_bytes(buff[..8].try_into().unwrap());
        buff = &buff[8..];

        let mut state = MaybeUninit::<CpuState>::uninit();
        buff = CpuState::decompress(buff, unsafe { state.assume_init_mut() })?;

        if buff.len() < 16 {
            return Err(CodecError);
        }

        let mut insn = [0u8; 15];
        insn.copy_from_slice(&buff[..15]);
        buff = &buff[15..];

        let size = buff[0];
        buff = &buff[1..];

        Ok((
            buff,
            Self {
                id,
                state: unsafe { state.assume_init() },
                insn,
                size,
            },
        ))
    }
}

/// A test result
#[derive(Debug, PartialEq, Eq)]
pub struct TestResult {
    /// The id of this test
    pub id: TestId,

    /// The test outcome
    pub outcome: TestOutcome,
}

impl TestResult {
    /// Writes a TestCase as a sequence of bytes to an address
    pub fn to_bytes<'a>(&self, mut buff: &'a mut [u8]) -> Result<&'a mut [u8], CodecError> {
        if buff.len() < 9 {
            return Err(CodecError);
        }

        buff[..8].copy_from_slice(&self.id.to_le_bytes());
        buff = &mut buff[8..];

        match &self.outcome {
            TestOutcome::Completed(diff) => {
                buff[0] = 0;
                buff = &mut buff[1..];
                buff = diff.to_bytes(buff)?;
            }
            TestOutcome::Exception(exception) => {
                if buff.len() < 18 {
                    return Err(CodecError);
                }

                buff[0] = 1;
                buff[1] = exception.kind as u8;
                buff[2] = exception.size;
                buff[3..18].copy_from_slice(&exception.insn);
                buff = &mut buff[18..];
            }
        }

        Ok(buff)
    }

    pub fn from_bytes(mut buff: &[u8]) -> Result<(&[u8], Self), CodecError> {
        if buff.len() < 9 {
            return Err(CodecError);
        }

        let id = usize::from_le_bytes(buff[..8].try_into().unwrap());
        buff = &buff[8..];

        let tag = buff[0];
        buff = &buff[1..];

        let outcome = match tag {
            0 => {
                let mut state = MaybeUninit::<CpuState>::uninit();
                buff = CpuState::decompress(buff, unsafe { state.assume_init_mut() })?;
                TestOutcome::Completed(unsafe { state.assume_init() })
            }
            1 => {
                if buff.len() < 17 {
                    return Err(CodecError);
                }

                let kind = ExceptionVector::try_from(buff[0])?;
                let size = buff[1];

                let mut insn = [0u8; 15];
                insn.copy_from_slice(&buff[2..17]);
                buff = &buff[17..];

                TestOutcome::Exception(ExceptionInfo { kind, insn, size })
            }
            _ => return Err(CodecError),
        };

        Ok((buff, Self { id, outcome }))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        cpu::CpuState,
        testcase::{ExceptionInfo, ExceptionVector, TestCase, TestOutcome, TestResult},
        *,
    };

    #[test]
    fn case_compress_decompress() {
        let mut state = CpuState::zero();
        state.gpr.rax = 15;
        state.fpu.initialize_mmx();
        state.fpu.set_mmx(3, 69);
        state.rip = 720;
        state.flags = cpu::FlagState(95);

        let start = TestCase {
            id: 42,
            state,
            insn: [5; 15],
            size: 3,
        };

        let mut buff = [0; 4096];

        let written = start
            .to_bytes(&mut buff)
            .expect("Error while writting")
            .as_ptr();

        let (read, end) = TestCase::from_bytes(&buff).expect("Error");

        assert_eq!(start, end);
        assert_eq!(written, read.as_ptr());
    }

    #[test]
    fn result_compress_decompress() {
        let mut state = CpuState::zero();
        state.gpr.rax = 15;
        state.fpu.initialize_mmx();
        state.fpu.set_mmx(3, 69);
        state.rip = 720;
        state.flags = cpu::FlagState(95);

        let start = TestResult {
            id: 42,
            outcome: TestOutcome::Completed(state),
        };

        let mut buff = [0; 4096];

        let written = start
            .to_bytes(&mut buff)
            .expect("Error while writting")
            .as_ptr();

        let (read, end) = TestResult::from_bytes(&buff).expect("Error");

        assert_eq!(start, end);
        assert_eq!(written, read.as_ptr());
    }

    #[test]
    fn exception_result_compress_decompress() {
        let start = TestResult {
            id: 69,
            outcome: TestOutcome::Exception(ExceptionInfo {
                kind: ExceptionVector::Double,
                insn: [0xCC; 15],
                size: 4,
            }),
        };

        let mut buff = [0; 4096];

        let written = start
            .to_bytes(&mut buff)
            .expect("Error while writting")
            .as_ptr();

        let (read, end) = TestResult::from_bytes(&buff).expect("Error");

        assert_eq!(start, end);
        assert_eq!(written, read.as_ptr());
    }
}
