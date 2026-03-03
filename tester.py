# input file fomat:
# ADC, 8, 8
# ADC, 16, 16
# MUL, 8
# For each instruction, generate a test case with the correct register: 8 -> al or ah, 16 -> ax, 32 -> eax, 64 -> rax.
# For each case, run it with values: all sets of REG_VALUES for rax and rbx and all sets of FLAG_VALUES
# Then for each result, store the mnemonic, values of rax, rbx, flags before and after, and the expected result in a file.

from pyaegis import PyAegis, PyCpuState, PyTest, PyNamedTestCase
import random
from keystone import Ks, KS_ARCH_X86, KS_MODE_64
from tqdm import tqdm

ks = Ks(KS_ARCH_X86, KS_MODE_64)


REG_VALUES = [
    0x00,
    0x01,
    0x02,
    0x03,
    0x3F,
    0x40,
    0x41,
    0x7E,
    0x7F,
    0x80,
    0x81,
    0x82,
    0xBF,
    0xC0,
    0xC1,
    0xFC,
    0xFD,
    0xFE,
    0xFF,
    #
    0xFF00,
    0xFF01,
    0xFF02,
    0xFF03,
    0xFF3F,
    0xFF40,
    0xFF41,
    0xFF7E,
    0xFF7F,
    0xFF80,
    0xFF81,
    0xFF82,
    0xFFBF,
    0xFFC0,
    0xFFC1,
    0xFFFC,
    0xFFFD,
    0xFFFE,
    0xFFFF,
    #
    0xFFFFFF00,
    0xFFFFFF01,
    0xFFFFFF02,
    0xFFFFFF03,
    0xFFFFFF3F,
    0xFFFFFF40,
    0xFFFFFF41,
    0xFFFFFF7E,
    0xFFFFFF7F,
    0xFFFFFF80,
    0xFFFFFF81,
    0xFFFFFF82,
    0xFFFFFFBF,
    0xFFFFFFC0,
    0xFFFFFFC1,
    0xFFFFFFFC,
    0xFFFFFFFD,
    0xFFFFFFFE,
    0xFFFFFFFF,
    #
    0xFFFFFFFFFF00,
    0xFFFFFFFFFF01,
    0xFFFFFFFFFF02,
    0xFFFFFFFFFF03,
    0xFFFFFFFFFF3F,
    0xFFFFFFFFFF40,
    0xFFFFFFFFFF41,
    0xFFFFFFFFFF7E,
    0xFFFFFFFFFF7F,
    0xFFFFFFFFFF80,
    0xFFFFFFFFFF81,
    0xFFFFFFFFFF82,
    0xFFFFFFFFFFBF,
    0xFFFFFFFFFFC0,
    0xFFFFFFFFFFC1,
    0xFFFFFFFFFFFC,
    0xFFFFFFFFFFFD,
    0xFFFFFFFFFFFE,
    0xFFFFFFFFFFFF,
]

FLAG_VALUES = [
    0x0000,  # No flags set
    0x0001,  # CF
    0x0004,  # PF
    0x0010,  # AF
    0x0040,  # ZF
    0x0080,  # SF
    0x0800,  # OF
    0x08D5,  # All flags set
]

# Input and output files
INPUT_FILE = "../pcode/src/tests/x64/insn.txt"
OUTPUT_FILE = "test_results.txt"

# These paths should match the server configuration
SERIAL_SOCK = "/tmp/serial.sock"
SHARED_MEM = "/dev/shm/ivshmem"

# Global state for test generation
test_queue = []
test_index = 0
pbar = None


def register_for_size(size: int, operand_index: int = 0) -> str:
    """Map operand size to register name. First operand uses rax, second uses rbx."""
    reg_base = "rax" if operand_index == 0 else "rbx"

    if size == 8:
        return "al" if operand_index == 0 else "bl"
    elif size == 16:
        return "ax" if operand_index == 0 else "bx"
    elif size == 32:
        return "eax" if operand_index == 0 else "ebx"
    elif size == 64:
        return "rax" if operand_index == 0 else "rbx"
    else:
        raise ValueError(f"Unsupported size: {size}")


def generate_test_queue() -> None:
    """Generate all test cases from input file"""
    global test_queue

    try:
        with open(INPUT_FILE, "r") as f:
            lines = [
                line.strip()
                for line in f
                if line.strip() and not line.strip().startswith("#")
            ]

        # Strip the first 130 lines (we already generated those tests)
        # lines = lines[129:]
        # assert lines[0].startswith("DEC,64"), "Unexpected first line after stripping"

        for line in tqdm(lines, desc="Generating test queue"):
            parts = [p.strip() for p in line.split(",")]
            mnemonic = parts[0]
            sizes = [int(p) for p in parts[1:]]

            # Generate all combinations of register values and flags
            for rax_val in REG_VALUES:
                for rbx_val in REG_VALUES:
                    for flags_val in FLAG_VALUES:
                        if len(sizes) == 1:
                            # Single operand instruction (e.g., MUL bl)
                            reg = register_for_size(sizes[0], 1)
                            asm_instr = f"{mnemonic.lower()} {reg}"
                        else:
                            # Multi-operand instruction (e.g., ADC al, bl)
                            regs = [
                                register_for_size(size, i)
                                for i, size in enumerate(sizes)
                            ]
                            asm_instr = f"{mnemonic.lower()} {', '.join(regs)}"

                        # Assemble instruction
                        try:
                            encoding, _ = ks.asm(asm_instr)
                            if encoding is None:
                                print(f"Failed to assemble instruction: {asm_instr}")
                                continue
                            encoding = bytes(encoding)
                        except Exception:
                            print(f"Error assembling instruction: {asm_instr}")
                            continue

                        test_queue.append(
                            {
                                "mnemonic": mnemonic,
                                "sizes": sizes,
                                "rax": rax_val,
                                "rbx": rbx_val,
                                "flags": flags_val,
                                "encoding": encoding,
                            }
                        )
    except FileNotFoundError:
        print(f"Input file {INPUT_FILE} not found")


def writer(id: int) -> PyNamedTestCase | None:
    """Generate test cases from queue"""
    global test_index, pbar

    # Initialize on first call
    if test_index == 0 and not test_queue:
        pbar = tqdm(total=len(test_queue), desc="Running tests")

    # Check if we've exhausted the queue
    if test_index >= len(test_queue):
        if pbar:
            pbar.close()
        return None

    test_spec = test_queue[test_index]
    test_index += 1

    if pbar:
        pbar.update(1)

    # Create CPU state with specified values
    state = PyCpuState()
    state.rax = test_spec["rax"]
    state.rbx = test_spec["rbx"]
    state.flags = test_spec["flags"]

    # Build assembly instruction

    test_name = f"{test_index}"
    return PyNamedTestCase(id, test_name, state, test_spec["encoding"])


def reader(res: PyTest) -> None:
    """Process and store test results"""
    result = {
        "test_id": res.id,
        "mnemonic": test_queue[res.id]["mnemonic"],
        "start_state": {
            "rax": res.start_state.rax,
            "rbx": res.start_state.rbx,
            "flags": res.start_state.flags.value,
        },
        "end_state": {
            "rax": res.end_state.rax,
            "rbx": res.end_state.rbx,
            "rdx": res.end_state.rdx,
            "flags": res.end_state.flags.value,
        },
    }

    # Write result to file
    with open(OUTPUT_FILE, "a") as f:
        f.write(str(result) + "\n")


def main():
    generate_test_queue()
    print(f"Generated {len(test_queue)} test cases")

    Aegis = PyAegis(
        SERIAL_SOCK,
        SHARED_MEM,
        reader,
        writer,
    )
    Aegis.run()


if __name__ == "__main__":
    main()
