#[cfg(not(feature = "std"))]
extern crate core;

#[cfg(not(feature = "std"))]
use core::mem::MaybeUninit;

#[cfg(feature = "std")]
use std::mem::MaybeUninit;

use crate::{compressible::Compressible, cpu::CpuState};

pub type TestId = usize;

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

        buff[..15].copy_from_slice(&self.insn);
        buff = &mut buff[15..];

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

    /// The diffed state
    pub diff: CpuState,
}

impl TestResult {
    /// Writes a TestCase as a sequence of bytes to an address
    pub fn to_bytes<'a>(&self, mut buff: &'a mut [u8]) -> Result<&'a mut [u8], ()> {
        if buff.len() < 8 {
            return Err(());
        }

        buff[..8].copy_from_slice(&self.id.to_le_bytes());
        buff = &mut buff[8..];

        buff = self.diff.to_bytes(buff)?;

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

        Ok((
            buff,
            Self {
                id,
                diff: unsafe { state.assume_init() },
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        cpu::CpuState,
        testcase::{TestCase, TestResult},
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
            diff: state,
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
