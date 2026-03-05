import os
import sys
from collections.abc import Iterator
from typing import Any, cast

sys.path.insert(0, "/home/jack/dev/x86db")
from db import open_db
from psycopg2.extras import execute_values

from pyaegis import PyAegis, PyCpuState, PyNamedTestCase, PyTest
from tqdm import tqdm

# These paths should match the server configuration
SERIAL_SOCK = "/tmp/serial.sock"
SHARED_MEM = "/dev/shm/ivshmem"

BATCH_SIZE = 10000

# Global state for test generation
test_iter: Iterator[dict[str, object]] | None = None
test_index = 0
active_tests: dict[int, dict[str, object]] = {}
db_con: Any = None

pending_normal: list[tuple[int, int, dict[str, int]]] = []
pending_exception: list[tuple[int, int, str | None]] = []

REG_VALUE_MASK = 0xFFFFFFFFFFFF


def _to_signed(v: int) -> int:
    return v if v < 2**63 else v - 2**64


def iter_test_specs_from_db(con) -> Iterator[dict]:
    with con.cursor() as cur:
        cur.execute(
            """
            SELECT tc.id, tc.instruction, tc.opcode, isv.state_index, isv.name, isv.value
            FROM test_cases tc
            JOIN initial_state_values isv ON isv.test_case_id = tc.id
            LEFT JOIN test_results tr
              ON tr.test_case_id = isv.test_case_id
             AND tr.state_index = isv.state_index
            WHERE tr.id IS NULL
            ORDER BY tc.id, isv.state_index, isv.name
            """
        )

        current_test_case_id: int | None = None
        current_instruction = ""
        current_encoding = b""
        current_state_index: int | None = None
        current_state: dict[str, int] = {}

        # Rows are ordered, so we can stream and emit one state payload at a time.
        for test_case_id, instruction, opcode, state_index, name, value in cur:
            is_new_state = (
                current_test_case_id is None
                or test_case_id != current_test_case_id
                or state_index != current_state_index
            )

            if is_new_state:
                if current_test_case_id is not None and current_state_index is not None:
                    yield {
                        "test_case_id": current_test_case_id,
                        "state_index": current_state_index,
                        "instruction": current_instruction,
                        "encoding": current_encoding,
                        "initial_state": current_state,
                    }

                if test_case_id != current_test_case_id:
                    current_test_case_id = test_case_id
                    current_instruction = instruction
                    current_encoding = bytes.fromhex(opcode)

                current_state_index = state_index
                current_state = {}

            current_state[name] = value

        if current_test_case_id is not None and current_state_index is not None:
            yield {
                "test_case_id": current_test_case_id,
                "state_index": current_state_index,
                "instruction": current_instruction,
                "encoding": current_encoding,
                "initial_state": current_state,
            }


def apply_state(cpu_state: PyCpuState, state_data: dict[str, int]) -> list[str]:
    reg_keys: list[str] = []
    for key, value in state_data.items():
        if key == "flag":
            cpu_state.flags = value
            continue

        setattr(cpu_state, key, value & 0xFFFFFFFFFFFFFFFF)
        reg_keys.append(key)

    return reg_keys


def normalize_state(state_data: dict[str, int]) -> dict[str, int]:
    normalized = {key: int(value) & REG_VALUE_MASK for key, value in state_data.items()}
    if "flag" not in normalized:
        normalized["flag"] = 0
    return normalized


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
    global db_con, pending_normal, pending_exception

    if not pending_normal and not pending_exception:
        return

    with db_con.cursor() as cur:
        if pending_normal:
            rows = execute_values(
                cur,
                "INSERT INTO test_results (test_case_id, state_index) VALUES %s RETURNING id, test_case_id, state_index",
                [(tc_id, si) for tc_id, si, _ in pending_normal],
                fetch=True,
            )
            id_map = {(tc_id, si): result_id for result_id, tc_id, si in rows}
            state_rows = []
            for tc_id, si, final_state in pending_normal:
                result_id = id_map[(tc_id, si)]
                for name, v in final_state.items():
                    state_rows.append((result_id, name, _to_signed(v)))
            execute_values(
                cur,
                "INSERT INTO result_state_values (test_result_id, name, value) VALUES %s",
                state_rows,
            )

        if pending_exception:
            execute_values(
                cur,
                "INSERT INTO test_results (test_case_id, state_index, exception_kind) VALUES %s",
                pending_exception,
            )

    db_con.commit()
    pending_normal.clear()
    pending_exception.clear()


def writer(id: int) -> PyNamedTestCase | None:
    """Generate test cases from queue"""
    global test_index

    if test_iter is None:
        raise RuntimeError("Test iterator is not initialized")

    try:
        test_spec = next(test_iter)
    except StopIteration:
        return None

    # Create CPU state with specified values
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
        "initial_state": normalize_state(initial_state),
    }

    test_name = f"{id}:{test_spec['test_case_id']}:{test_spec['state_index']}"
    return PyNamedTestCase(id, test_name, state, encoding)


def reader(res: PyTest) -> None:
    """Process and store test results"""
    global active_tests, pending_normal, pending_exception

    test_meta = active_tests.pop(res.id, None)
    if test_meta is None:
        return

    test_case_id = int(cast(int, test_meta["test_case_id"]))
    state_index = int(cast(int, test_meta["state_index"]))

    if res.end_state is None:
        exception_kind = getattr(res, "exception_kind", None)
        pending_exception.append(
            (
                test_case_id,
                state_index,
                str(exception_kind) if exception_kind is not None else None,
            )
        )
    else:
        final_state = serialize_state(
            res.end_state,
            cast(list[str], test_meta["reg_keys"]),
            include_rdx=True,
        )
        pending_normal.append((test_case_id, state_index, final_state))

    if len(pending_normal) + len(pending_exception) >= BATCH_SIZE:
        flush_results()


def main():
    global test_iter, test_index, active_tests, db_con

    db_con = open_db(
        os.environ.get("X86DB_DSN", "postgresql://x86db:x86db@localhost:5432/x86db")
    )

    with db_con.cursor() as cur:
        cur.execute(
            """
            SELECT COUNT(*) FROM initial_state_values isv
            LEFT JOIN test_results tr ON tr.test_case_id = isv.test_case_id AND tr.state_index = isv.state_index
            WHERE tr.id IS NULL
        """
        )
        total = cur.fetchone()[0]

    test_iter = cast(
        Iterator[dict[str, object]],
        iter(tqdm(iter_test_specs_from_db(db_con), total=total)),
    )
    test_index = 0
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
