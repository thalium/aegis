from pyaegis import PyAegis, PyCpuState, PyTest, PyNamedTestCase
import random

# These paths should match the server configuration
SERIAL_SOCK = "/tmp/serial.sock"
SHARED_MEM = "/dev/shm/ivshmem"


# Use the results
def reader(res: PyTest) -> None:
    exception_kind = getattr(res, "exception_kind", None)
    exception_insn = getattr(res, "exception_insn", [])

    if exception_kind is not None:
        print(f"exception={exception_kind} insn={bytes(exception_insn).hex()}")
        return

    assert res.end_state is not None

    # Run any test with results
    assert (
        res.end_state.rax
        == (res.start_state.rax + res.start_state.rbx) & 0xFFFFFFFFFFFFFFFF
    )


def writer(id: int) -> PyNamedTestCase | None:
    # Quit after 10 tests
    if id > 10:
        # Returning None ends testing
        return None

    # Create a blank cpu state
    state = PyCpuState()

    # Give rax and rbx random values
    state.rax = int.from_bytes(random.randbytes(8))
    state.rbx = int.from_bytes(random.randbytes(8))

    # Assemble instruction
    try:
        from keystone import Ks, KS_ARCH_X86, KS_MODE_64

        ks = Ks(KS_ARCH_X86, KS_MODE_64)
        encoding, _ = ks.asm("add rax, rbx")
        assert encoding is not None
        encoding = bytes(encoding)
    except ImportError:
        # You can use any other method to assemble instructions
        encoding = b"\x48\x01\xd8"

    return PyNamedTestCase(id, "add_rax_rbx", state, encoding)


def main():
    Aegis = PyAegis(
        SERIAL_SOCK,
        SHARED_MEM,
        reader,
        writer,
    )
    Aegis.run()


if __name__ == "__main__":
    main()
