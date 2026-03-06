import os
import sys
from collections.abc import Iterator
from typing import Any, cast

sys.path.insert(0, "/home/jack/dev/x86db")
from db2 import CpuState, TestResult, bulk_upsert_results, iter_test_cases, open_db

from pyaegis import PyAegis, PyCpuState, PyNamedTestCase, PyTest
from tqdm import tqdm

# These paths should match the server configuration
SERIAL_SOCK = "/tmp/serial.sock"
SHARED_MEM = "/dev/shm/ivshmem"

BATCH_SIZE = 10000

DSN = os.environ.get("X86DB_DSN", "postgresql://x86db:x86db@localhost:5432/x86db")

# Global state for test generation
test_iter: Iterator[dict[str, object]] | None = None
active_tests: dict[int, dict[str, object]] = {}
db_con: Any = None

pending: list[TestResult] = []

REG_VALUE_MASK = 0xFFFF_FFFF_FFFF_FFFF


def iter_test_specs_from_db(dsn: str) -> Iterator[dict]:
    with open_db(dsn) as conn:
        with conn.cursor() as cur:
            cur.execute("SELECT test_case_id, state_index FROM test_results")
            completed: set[tuple[int, int]] = set(cur.fetchall())

    for tc in iter_test_cases(dsn):
        encoding = bytes.fromhex(tc.opcode)
        for state_index, cpu_state in enumerate(tc.initial_states):
            if (tc.id, state_index) not in completed:
                yield {
                    "test_case_id": tc.id,
                    "state_index": state_index,
                    "instruction": tc.instruction,
                    "encoding": encoding,
                    "initial_state": cpu_state.regs,
                }


def apply_state(cpu_state: PyCpuState, state_data: dict[str, int]) -> list[str]:
    reg_keys: list[str] = []
    for key, value in state_data.items():
        if key == "flag":
            cpu_state.flags = value
            continue

        setattr(cpu_state, key, value & REG_VALUE_MASK)
        reg_keys.append(key)

    return reg_keys


def serialize_state(
    cpu_state: PyCpuState, reg_keys: list[str], include_rdx: bool = False
) -> dict[str, int]:
    keys = list(dict.fromkeys(reg_keys))
    if include_rdx and "rdx" not in keys:
        keys.append("rdx")

    state_dict: dict[str, int] = {
        reg: int(getattr(cpu_state, reg)) & REG_VALUE_MASK for reg in keys
    }
    state_dict["flag"] = int(cpu_state.flags.value) & REG_VALUE_MASK
    return state_dict


def flush_results() -> None:
    global db_con, pending

    if not pending:
        return

    bulk_upsert_results(db_con, pending)
    db_con.commit()
    pending.clear()


def writer(id: int) -> PyNamedTestCase | None:
    """Generate test cases from queue"""
    if test_iter is None:
        raise RuntimeError("Test iterator is not initialized")

    try:
        test_spec = next(test_iter)
    except StopIteration:
        return None

    state = PyCpuState()
    initial_state = cast(dict[str, int], test_spec["initial_state"])
    encoding = cast(bytes, test_spec["encoding"])
    reg_keys = apply_state(state, initial_state)

    active_tests[id] = {
        "test_case_id": test_spec["test_case_id"],
        "state_index": test_spec["state_index"],
        "instruction": test_spec["instruction"],
        "encoding": encoding,
        "reg_keys": reg_keys,
    }

    test_name = f"{id}:{test_spec['test_case_id']}:{test_spec['state_index']}"
    return PyNamedTestCase(id, test_name, state, encoding)


def reader(res: PyTest) -> None:
    """Process and store test results"""
    global active_tests, pending

    test_meta = active_tests.pop(res.id, None)
    if test_meta is None:
        return

    test_case_id = int(cast(int, test_meta["test_case_id"]))
    state_index = int(cast(int, test_meta["state_index"]))

    if res.end_state is None:
        exception_kind = getattr(res, "exception_kind", None)
        pending.append(
            TestResult(
                test_case_id=test_case_id,
                state_index=state_index,
                exception_kind=(
                    str(exception_kind) if exception_kind is not None else None
                ),
                final_state=None,
            )
        )
    else:
        final_state = serialize_state(
            res.end_state,
            cast(list[str], test_meta["reg_keys"]),
            include_rdx=True,
        )
        pending.append(
            TestResult(
                test_case_id=test_case_id,
                state_index=state_index,
                exception_kind=None,
                final_state=CpuState(regs=final_state),
            )
        )

    if len(pending) >= BATCH_SIZE:
        flush_results()


def main():
    global test_iter, active_tests, db_con

    db_con = open_db(DSN)

    with db_con.cursor() as cur:
        cur.execute(
            """
            SELECT
                (SELECT SUM(jsonb_array_length(initial_states)) FROM test_cases) -
                (SELECT COUNT(*) FROM test_results)
        """
        )
        total = cur.fetchone()[0]

    test_iter = cast(
        Iterator[dict[str, object]],
        iter(tqdm(iter_test_specs_from_db(DSN), total=total)),
    )
    active_tests = {}

    Aegis = PyAegis(
        SERIAL_SOCK,
        SHARED_MEM,
        reader,
        writer,
        quiet=True,
    )
    Aegis.run()
    flush_results()


if __name__ == "__main__":
    main()
