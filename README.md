# Aegis x86_64

Aegis is a testing framework for x86_64 IR lifters.
It allows precise inspection of CPU state before and after a single instruction is executed, enabling unit testing of your lifter.

## Requirements

- Linux with QEMU + KVM support.
- CPU support for AVX and XSave.
- QEMU with [ivshmem](https://www.qemu.org/docs/master/system/devices/ivshmem.html) support.

Note: Nested KVM environments may not work (e.g., running inside another VM).

## Installation

### Client library

Install the Python client library via pip:

```bash
pip install aegis-x86
```

### Server boot image (Docker)

Build the boot image with Docker:

```bash
docker build -o build .
```

This produces `bootimage-aegis.bin`.

### Development build (Rust)

Aegis uses Rust nightly (`rustc 1.94.0-nightly`) for building the kernel and libraries.
Required components:

```
rustup component add llvm-tools-preview rust-src --toolchain nightly-x86_64-unknown-linux-gnu
cargo install bootimage just
```
You can use `just` recipes to build the boot image, client, and Python bindings.

## Usage

### Running the server and client

```bash
qemu-system-x86_64                                          \
    -drive format=raw,file=./build/bootimage-aegis.bin      \
    -enable-kvm                                             \
    -cpu qemu64,kvm=on,+xsave,+avx                          \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04          \
    -serial unix:/tmp/serial.sock,server,nowait             \
    -device ivshmem-plain,memdev=ivmem                      \
    -object memory-backend-file,id=ivmem,share=on,mem-path=/dev/shm/ivshmem,size=16M &
python client.py
```

## Writing your own client

A client must provide two callbacks:

- **Reader** (`reader(PyTest) -> None`)
    Recieves the instruction execution results and CPU state before/after execution.
    When an exception occurs during a test, `res.exception_kind` and
    `res.exception_insn` are populated and `res.end_state` is `None`.
  Here's where you add your own lifter's code

- **Writer** (`writer(int) -> PyNamedTestCase | None`)
    Genereates test cases for the server. Returning `None` ends testing.

Example client (`client.py`):

```py
from pyaegis import PyAegis, PyCpuState, PyTest, PyNamedTestCase
import random

# These paths should match the server configuration
SERIAL_SOCK = "/tmp/serial.sock"
SHARED_MEM = "/dev/shm/ivshmem"


# Use the results
def reader(res: PyTest) -> None:
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

```

## Architecture

Aegis is split into two main components:
- **Client**: runs on the host, generates instructions, receives results, and executes tests.
- **Server**: a custom Rust microkernel running inside QEMU KVM; executes instructions and inspects CPU state.

Communication is via shared memory (ivshmem) and serial port.
Tests are run fully independently.

## Acknowledgements

The rust micro kernel was inspired by [Writing an OS in Rust](https://os.phil-opp.com/).

## Similar projects

- https://github.com/ZehMatt/x86Tester
- https://github.com/phorcys/x64_inst_test
