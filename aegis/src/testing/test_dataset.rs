use libaegis::{
    cpu::CpuState,
    testcase::{ExceptionInfo, TestCase, TestOutcome, TestResult},
};

use crate::testing::{
    harness::{Dataset, TestId},
    shared_memory::SHARED_MEMORY_MANAGER,
};

static mut INITIAL_CPU_STATE: CpuState = CpuState::zero();

/// A zero sized struct describing our test
pub struct TestDataset;

#[allow(static_mut_refs)]
impl Dataset for TestDataset {
    /// Ideally this would be a generator or access a table
    /// here, we always return an add instruction
    fn next(&self) -> TestCase {
        let mut shared_mem_manager = SHARED_MEMORY_MANAGER.lock();
        let buff = shared_mem_manager.read_buffer();

        let (buff, test) = match TestCase::from_bytes(buff) {
            Ok((buff, test)) => (buff, test),
            Err(()) => {
                shared_mem_manager.refresh_read_buffer();
                let buff = shared_mem_manager.read_buffer();
                TestCase::from_bytes(buff).expect("Failed to read despite request")
            }
        };

        shared_mem_manager.set_read_buffer(buff);

        unsafe {
            INITIAL_CPU_STATE = test.state.clone();
        }

        test
    }

    /// Here we can save / inspect test results
    fn after_test(&mut self, id: TestId, state: &CpuState, exception: Option<ExceptionInfo>) {
        let outcome = match exception {
            Some(exception) => TestOutcome::Exception(exception),
            None => {
                let diff = state.diff(unsafe { &INITIAL_CPU_STATE });
                TestOutcome::Completed(diff)
            }
        };

        let res = TestResult { id, outcome };

        let mut shared_mem_manager = SHARED_MEMORY_MANAGER.lock();
        let buff = shared_mem_manager.write_buffer();

        match res.to_bytes(buff) {
            Err(_) => {
                shared_mem_manager.clear_write_buffer();
                let buff = shared_mem_manager.write_buffer();
                shared_mem_manager.set_write_buffer(
                    res.to_bytes(buff)
                        .expect("Unable to write to write buffer even after clear"),
                );
            }

            Ok(buff) => shared_mem_manager.set_write_buffer(buff),
        }

        // Clear after the first read
        if id == 0 {
            shared_mem_manager.clear_write_buffer();
        }
    }
}
