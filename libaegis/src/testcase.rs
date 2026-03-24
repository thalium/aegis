#[cfg(not(feature = "std"))]
extern crate core;

#[cfg(not(feature = "std"))]
use core::mem::MaybeUninit;

#[cfg(feature = "std")]
use std::mem::MaybeUninit;

use crate::{compressible::Compressible, cpu::CpuState};

pub type TestId = usize;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionKind {
    Unknown = 0,
    PageFault = 1,
    DoubleFault = 2,
    GeneralProtection = 3,
    InvalidOpcode = 4,
}

impl ExceptionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::PageFault => "PageFault",
            Self::DoubleFault => "DoubleFault",
            Self::GeneralProtection => "GeneralProtection",
            Self::InvalidOpcode => "InvalidOpcode",
        }
    }
}

impl TryFrom<u8> for ExceptionKind {
    type Error = ();

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::PageFault),
            2 => Ok(Self::DoubleFault),
            3 => Ok(Self::GeneralProtection),
            4 => Ok(Self::InvalidOpcode),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionInfo {
    pub kind: ExceptionKind,
    pub insn: [u8; 15],
    pub size: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub fn to_bytes<'a>(&self, mut buff: &'a mut [u8]) -> Result<&'a mut [u8], ()> {
        if buff.len() < 8 {
            return Err(());
        }

        buff[..8].copy_from_slice(&self.id.to_le_bytes());
        buff = &mut buff[8..];

        buff = self.state.to_bytes(buff)?;

        if buff.len() < 16 {
            return Err(());
        }

        buff[..15].copy_from_slice(&self.insn);
        buff = &mut buff[15..];

        if buff.is_empty() {
            return Err(());
        }

        buff[0] = self.size;
        buff = &mut buff[1..];

        Ok(buff)
    }

    pub fn from_bytes<'a>(mut buff: &'a [u8]) -> Result<(&'a [u8], Self), ()> {
        if buff.len() < 8 {
            return Err(());
        }

        let id = usize::from_le_bytes(buff[..8].try_into().unwrap());
        buff = &buff[8..];

        let mut state = MaybeUninit::<CpuState>::uninit();
        buff = CpuState::decompress(buff, unsafe { state.assume_init_mut() })?;

        if buff.len() < 16 {
            return Err(());
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
    pub fn to_bytes<'a>(&self, mut buff: &'a mut [u8]) -> Result<&'a mut [u8], ()> {
        if buff.len() < 9 {
            return Err(());
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
                    return Err(());
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

    pub fn from_bytes<'a>(mut buff: &'a [u8]) -> Result<(&'a [u8], Self), ()> {
        if buff.len() < 9 {
            return Err(());
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
                    return Err(());
                }

                let kind = ExceptionKind::try_from(buff[0])?;
                let size = buff[1];

                let mut insn = [0u8; 15];
                insn.copy_from_slice(&buff[2..17]);
                buff = &buff[17..];

                TestOutcome::Exception(ExceptionInfo { kind, insn, size })
            }
            _ => return Err(()),
        };

        Ok((buff, Self { id, outcome }))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        cpu::CpuState,
        testcase::{ExceptionInfo, ExceptionKind, TestCase, TestOutcome, TestResult},
        *,
    };

    #[test]
    fn case_compress_decompress() {
        let mut state = CpuState::zero();
        state.gpr.rax = 15;
        state.mmx.mm3 = 69;
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
        state.mmx.mm3 = 69;
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
                kind: ExceptionKind::DoubleFault,
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
