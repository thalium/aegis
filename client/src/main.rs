use client::{Aegis, AegisConfig, NamedTestCase, Test};
use libaegis::{cpu::CpuState, testcase::TestCase};

const SERIAL_SOCK: &str = "/tmp/serial.sock";
const SHARED_MEM: &str = "/dev/shm/ivshmem";

fn read(result: Test) {
    assert_eq!(
        result.start_state.gpr.rax + result.start_state.gpr.rbx,
        result.end_state.gpr.rax
    );

    assert_eq!(result.start_state.gpr.rbx, result.end_state.gpr.rbx);
}

fn write(id: usize) -> Option<NamedTestCase> {
    let mut state = CpuState::zero();
    state.gpr.rax = rand::random();
    state.gpr.rbx = rand::random();

    if id > 1_000_000 {
        return None;
    }

    Some(NamedTestCase {
        name: format!("Test {}", id),
        test_case: TestCase {
            id,
            insn: [0x48, 0x01, 0xd8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            size: 3,
            state,
        },
    })
}

fn main() {
    let mut aegis = Aegis::new(AegisConfig {
        serial_sock: SERIAL_SOCK,
        shared_mem: SHARED_MEM,
        ..Default::default()
    });

    aegis.init();
    aegis.set_read_executor(read);
    aegis.set_write_executor(write);
    aegis.run();
}
