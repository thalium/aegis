use std::{
    collections::{HashMap, HashSet, VecDeque},
    error::Error,
    sync::{Arc, Mutex},
};

mod aegis;

use aegis::{Aegis, NamedTestCase, Test};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use libaegis::{
    cpu::{CpuState, FlagState, SCRATCH_MEMORY_SIZE},
    testcase::TestCase,
};
use postgres::{Client as PgClient, NoTls, types::Json};
use serde_json::{Map, Number, Value};

const REG_VALUE_MASK: u64 = u64::MAX;

#[derive(Parser)]
#[command(about = "Run x86 instruction test cases against Aegis")]
struct Args {
    /// PostgreSQL DSN
    #[arg(
        long,
        env = "X86DB_DSN",
        default_value = "postgresql://x86db:x86db@localhost:5432/x86db"
    )]
    dsn: String,

    /// Serial socket path
    #[arg(long, default_value = "/tmp/serial.sock")]
    serial_sock: String,

    /// Shared memory path
    #[arg(long, default_value = "/dev/shm/ivshmem")]
    shared_mem: String,

    /// Number of results to batch before flushing to the database
    #[arg(long, default_value_t = 10_000)]
    batch_size: usize,

    /// Number of test cases to fetch per database query
    #[arg(long, default_value_t = 5_000)]
    fetch_cases: i64,

    /// Re-run all test cases, ignoring existing results in the database
    #[arg(long)]
    ignore_completed: bool,

    /// Restrict the run to these test-case IDs. May be passed more than once.
    #[arg(long = "test-case-id")]
    test_case_ids: Vec<i64>,
}

type DbState = Map<String, Value>;

#[derive(Clone)]
struct TestSpec {
    test_case_id: i64,
    state_index: i32,
    encoding: Vec<u8>,
    initial_state: DbState,
}

struct ActiveTest {
    test_case_id: i64,
    state_index: i32,
    reg_keys: Vec<String>,
}

struct PendingResult {
    test_case_id: i64,
    state_index: i32,
    exception_kind: Option<String>,
    final_state: Option<Value>,
}

/// State accessed by the writer: has its own DB connection and mutex,
/// so fetch operations never block the reader.
struct FetchState {
    db: PgClient,
    completed: HashSet<(i64, i32)>,
    buffered_specs: VecDeque<TestSpec>,
    last_test_case_id: i64,
    exhausted: bool,
    fetch_cases: i64,
    test_case_ids: Vec<i64>,
}

/// State accessed by the reader: flushing results to DB.
struct RunnerState {
    db: PgClient,
    active_tests: HashMap<usize, ActiveTest>,
    pending: Vec<PendingResult>,
    progress: ProgressBar,
    batch_size: usize,
}

fn json_u64(value: u64) -> Value {
    Value::Number(Number::from(value & REG_VALUE_MASK))
}

/// x87 registers are encoded as exactly ten little-endian bytes, represented
/// in JSON as twenty hexadecimal digits. Numeric JSON values are deliberately
/// not accepted: they cannot retain every raw 80-bit encoding.
fn x87_logical_register_index(key: &str) -> Option<usize> {
    key.strip_prefix("x87_st")?
        .parse::<usize>()
        .ok()
        .filter(|&index| index < 8)
}

/// `x87_rN` names canonical physical FPU register R(N), unlike the legacy
/// TOP-mapped `x87_stN` logical view.
fn x87_physical_register_index(key: &str) -> Option<usize> {
    key.strip_prefix("x87_r")?
        .parse::<usize>()
        .ok()
        .filter(|&index| index < 8)
}

fn value_to_x87(value: &Value) -> Result<[u8; 10], Box<dyn Error>> {
    let Value::String(text) = value else {
        return Err(format!(
            "x87 register value must be a 20-digit hexadecimal string, got {value}"
        )
        .into());
    };
    let text = text.strip_prefix("0x").unwrap_or(text);
    if text.len() != 20 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "invalid x87 register encoding {text:?}; expected 20 hexadecimal digits"
        )
        .into());
    }
    let mut out = [0; 10];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)?;
    }
    Ok(out)
}

fn json_x87(value: &[u8; 10]) -> Value {
    Value::String(value.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Raw bytes at the bounded `mem0` scratch range. Unlike `mem0_value`, this
/// preserves environment images and f80 payloads beyond the first word.
fn value_to_scratch_memory(value: &Value) -> Result<[u8; SCRATCH_MEMORY_SIZE], Box<dyn Error>> {
    let Value::String(text) = value else {
        return Err(format!(
            "scratch_memory must be a {}-digit lowercase hexadecimal string, got {value}",
            SCRATCH_MEMORY_SIZE * 2
        )
        .into());
    };
    if text.len() != SCRATCH_MEMORY_SIZE * 2
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "invalid scratch_memory; expected {} lowercase hexadecimal digits",
            SCRATCH_MEMORY_SIZE * 2
        )
        .into());
    }
    let mut out = [0; SCRATCH_MEMORY_SIZE];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)?;
    }
    Ok(out)
}

fn json_scratch_memory(value: &[u8; SCRATCH_MEMORY_SIZE]) -> Value {
    Value::String(value.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn value_to_u64(value: &Value) -> Result<u64, Box<dyn Error>> {
    match value {
        Value::Number(n) => {
            if let Some(v) = n.as_u64() {
                Ok(v)
            } else if let Some(v) = n.as_i64() {
                Ok(v as u64)
            } else {
                Err(format!("unsupported floating-point state value: {n}").into())
            }
        }
        Value::String(s) => Ok(s.parse::<u64>()?),
        other => Err(format!("unsupported state value: {other}").into()),
    }
}

fn state_array(value: Value, test_case_id: i64) -> Result<Vec<DbState>, Box<dyn Error>> {
    match value {
        Value::Array(states) => states
            .into_iter()
            .map(|state| match state {
                Value::Object(map) => Ok(map),
                other => {
                    Err(format!("test case {test_case_id} has non-object state {other}").into())
                }
            })
            .collect(),
        other => {
            Err(format!("test case {test_case_id} has non-array initial_states {other}").into())
        }
    }
}

fn fetch_completed(db: &mut PgClient) -> Result<HashSet<(i64, i32)>, Box<dyn Error>> {
    let mut completed = HashSet::new();
    for row in db.query("SELECT test_case_id, state_index FROM test_results", &[])? {
        completed.insert((row.get(0), row.get(1)));
    }
    Ok(completed)
}

fn fetch_next_specs(state: &mut FetchState) -> Result<(), Box<dyn Error>> {
    while state.buffered_specs.is_empty() && !state.exhausted {
        let rows = if state.test_case_ids.is_empty() {
            state.db.query(
                "
                SELECT id, opcode, initial_states
                FROM test_cases
                WHERE id > $1
                ORDER BY id
                LIMIT $2
                ",
                &[&state.last_test_case_id, &state.fetch_cases],
            )?
        } else {
            state.db.query(
                "
                SELECT id, opcode, initial_states
                FROM test_cases
                WHERE id > $1 AND id = ANY($2)
                ORDER BY id
                LIMIT $3
                ",
                &[
                    &state.last_test_case_id,
                    &state.test_case_ids,
                    &state.fetch_cases,
                ],
            )?
        };

        if rows.is_empty() {
            state.exhausted = true;
            break;
        }

        for row in rows {
            let test_case_id: i64 = row.get("id");
            let opcode: String = row.get("opcode");
            let initial_states: Json<Value> = row.get("initial_states");

            state.last_test_case_id = test_case_id;

            if opcode.starts_with("c8") {
                continue;
            }

            let encoding = hex::decode(&opcode)?;
            for (state_index, initial_state) in state_array(initial_states.0, test_case_id)?
                .into_iter()
                .enumerate()
            {
                let state_index = state_index as i32;
                if state.completed.contains(&(test_case_id, state_index)) {
                    continue;
                }

                state.buffered_specs.push_back(TestSpec {
                    test_case_id,
                    state_index,
                    encoding: encoding.clone(),
                    initial_state,
                });
            }
        }
    }

    Ok(())
}

fn set_vector<const N: usize>(bytes: &mut [u8; N], value: u64) {
    bytes[..8].copy_from_slice(&value.to_le_bytes());
}

fn apply_state(
    cpu_state: &mut CpuState,
    state_data: &DbState,
) -> Result<Vec<String>, Box<dyn Error>> {
    let has_logical_x87 = state_data
        .keys()
        .any(|key| x87_logical_register_index(key).is_some());
    let has_physical_x87 = state_data
        .keys()
        .any(|key| x87_physical_register_index(key).is_some());
    if has_logical_x87 && has_physical_x87 {
        return Err("x87_stN logical and x87_rN physical fields cannot be mixed".into());
    }
    if state_data.contains_key("scratch_memory")
        && (state_data.contains_key("mem0_value") || state_data.contains_key("mem1_value"))
    {
        return Err("scratch_memory cannot be mixed with mem0_value or mem1_value".into());
    }
    let has_x87 = has_logical_x87
        || has_physical_x87
        || state_data.keys().any(|key| {
            matches!(
                key.as_str(),
                "x87_control"
                    | "x87_status"
                    | "x87_top"
                    | "x87_tag"
                    | "x87_opcode"
                    | "x87_ip"
                    | "x87_dp"
            )
        });
    let has_mmx = state_data.keys().any(|key| {
        matches!(
            key.as_str(),
            "mm0" | "mm1" | "mm2" | "mm3" | "mm4" | "mm5" | "mm6" | "mm7"
        )
    });
    // An MMX-only row uses the historical active-MMX initialization. A mixed
    // row writes the same canonical file through both views instead.
    if has_mmx && !has_x87 {
        cpu_state.fpu.initialize_mmx();
    }
    // Set TOP before mapping any legacy logical ST(i) fields; JSON object
    // ordering must not affect which physical FPU slot receives a value.
    if let Some(status) = state_data.get("x87_status") {
        cpu_state.fpu.status = (value_to_u64(status)? & REG_VALUE_MASK).try_into()?;
    }
    if let Some(top) = state_data.get("x87_top") {
        let top = value_to_u64(top)?;
        if top > 7 {
            return Err(format!("x87_top must be in 0..=7, got {top}").into());
        }
        if let Some(status) = state_data.get("x87_status") {
            let status_top = (value_to_u64(status)? >> 11) & 7;
            if status_top != top {
                return Err(format!(
                    "x87_top ({top}) conflicts with x87_status.TOP ({status_top})"
                )
                .into());
            }
        }
        cpu_state.fpu.status = (cpu_state.fpu.status & !(7 << 11)) | ((top as u16) << 11);
    }

    // Mixed x87/MMX JSON must agree before either view is written, otherwise
    // object iteration order would decide the physical register contents.
    if has_x87 && has_mmx {
        for physical_index in 0..8 {
            let mmx_key = format!("mm{physical_index}");
            let Some(mmx_value) = state_data.get(&mmx_key) else {
                continue;
            };
            let x87_key = if has_physical_x87 {
                format!("x87_r{physical_index}")
            } else {
                let logical_index = (physical_index + 8 - cpu_state.fpu.top()) & 7;
                format!("x87_st{logical_index}")
            };
            let Some(x87_value) = state_data.get(&x87_key) else {
                continue;
            };
            let x87_value = value_to_x87(x87_value)?;
            if u64::from_le_bytes(x87_value[..8].try_into()?)
                != value_to_u64(mmx_value)? & REG_VALUE_MASK
            {
                return Err(format!(
                    "{mmx_key} conflicts with {x87_key}: both name physical FPU slot {physical_index}"
                )
                .into());
            }
        }
    }

    let mut reg_keys = Vec::new();

    for (key, value) in state_data {
        if key == "scratch_memory" {
            cpu_state.scratch_memory = value_to_scratch_memory(value)?;
            cpu_state.scratch_memory_len = SCRATCH_MEMORY_SIZE as u16;
            reg_keys.push(key.clone());
            continue;
        }
        if let Some(index) = x87_logical_register_index(key) {
            cpu_state.fpu.set_st(index, value_to_x87(value)?);
            reg_keys.push(key.clone());
            continue;
        }
        if let Some(index) = x87_physical_register_index(key) {
            cpu_state.fpu.registers[index] = value_to_x87(value)?;
            reg_keys.push(key.clone());
            continue;
        }

        let value = value_to_u64(value)? & REG_VALUE_MASK;
        match key.as_str() {
            "x87_control" => {
                cpu_state.fpu.control = value.try_into()?;
                reg_keys.push(key.clone());
            }
            "x87_status" => {
                cpu_state.fpu.status = value.try_into()?;
                reg_keys.push(key.clone());
            }
            // TOP is bits 11..13 of x87_status. Accept this redundant input
            // only for validation/legacy fixtures; do not record it in output.
            "x87_top" => {}
            "x87_tag" => {
                // `x87_tag` is the physical-order FXSAVE abridged tag byte
                // for the physical x87_rN transport. Preserve the legacy
                // logical interpretation only for x87_stN compatibility.
                if has_logical_x87 {
                    cpu_state.fpu.set_logical_tag(value.try_into()?);
                } else {
                    cpu_state.fpu.tag = value.try_into()?;
                }
                reg_keys.push(key.clone());
            }
            "x87_opcode" => {
                cpu_state.fpu.opcode = value.try_into()?;
                reg_keys.push(key.clone());
            }
            "x87_ip" => {
                cpu_state.fpu.instruction_pointer = value;
                reg_keys.push(key.clone());
            }
            "x87_dp" => {
                cpu_state.fpu.data_pointer = value;
                reg_keys.push(key.clone());
            }
            "flag" => cpu_state.flags = FlagState(value),
            "mem0_value" => {
                cpu_state.mem0 = value;
                reg_keys.push(key.clone());
            }
            "mem1_value" => {
                cpu_state.mem1 = value;
                reg_keys.push(key.clone());
            }
            "mm0" => {
                cpu_state.fpu.set_mmx(0, value);
                reg_keys.push(key.clone());
            }
            "mm1" => {
                cpu_state.fpu.set_mmx(1, value);
                reg_keys.push(key.clone());
            }
            "mm2" => {
                cpu_state.fpu.set_mmx(2, value);
                reg_keys.push(key.clone());
            }
            "mm3" => {
                cpu_state.fpu.set_mmx(3, value);
                reg_keys.push(key.clone());
            }
            "mm4" => {
                cpu_state.fpu.set_mmx(4, value);
                reg_keys.push(key.clone());
            }
            "mm5" => {
                cpu_state.fpu.set_mmx(5, value);
                reg_keys.push(key.clone());
            }
            "mm6" => {
                cpu_state.fpu.set_mmx(6, value);
                reg_keys.push(key.clone());
            }
            "mm7" => {
                cpu_state.fpu.set_mmx(7, value);
                reg_keys.push(key.clone());
            }
            "rax" => {
                cpu_state.gpr.rax = value;
                reg_keys.push(key.clone());
            }
            "rbx" => {
                cpu_state.gpr.rbx = value;
                reg_keys.push(key.clone());
            }
            "rcx" => {
                cpu_state.gpr.rcx = value;
                reg_keys.push(key.clone());
            }
            "rdx" => {
                cpu_state.gpr.rdx = value;
                reg_keys.push(key.clone());
            }
            "rsi" => {
                cpu_state.gpr.rsi = value;
                reg_keys.push(key.clone());
            }
            "rdi" => {
                cpu_state.gpr.rdi = value;
                reg_keys.push(key.clone());
            }
            "rbp" => {
                cpu_state.gpr.rbp = value;
                reg_keys.push(key.clone());
            }
            "rsp" => {
                cpu_state.gpr.rsp = value;
                reg_keys.push(key.clone());
            }
            "r8" => {
                cpu_state.gpr.r8 = value;
                reg_keys.push(key.clone());
            }
            "r9" => {
                cpu_state.gpr.r9 = value;
                reg_keys.push(key.clone());
            }
            "r10" => {
                cpu_state.gpr.r10 = value;
                reg_keys.push(key.clone());
            }
            "r11" => {
                cpu_state.gpr.r11 = value;
                reg_keys.push(key.clone());
            }
            "r12" => {
                cpu_state.gpr.r12 = value;
                reg_keys.push(key.clone());
            }
            "r13" => {
                cpu_state.gpr.r13 = value;
                reg_keys.push(key.clone());
            }
            "r14" => {
                cpu_state.gpr.r14 = value;
                reg_keys.push(key.clone());
            }
            "r15" => {
                cpu_state.gpr.r15 = value;
                reg_keys.push(key.clone());
            }
            "rip" => {
                cpu_state.rip = value;
                reg_keys.push(key.clone());
            }
            "cs" => {
                cpu_state.seg.cs = value;
                reg_keys.push(key.clone());
            }
            "ds" => {
                cpu_state.seg.ds = value;
                reg_keys.push(key.clone());
            }
            "es" => {
                cpu_state.seg.es = value;
                reg_keys.push(key.clone());
            }
            "fs" => {
                cpu_state.seg.fs = value;
                reg_keys.push(key.clone());
            }
            "gs" => {
                cpu_state.seg.gs = value;
                reg_keys.push(key.clone());
            }
            "ss" => {
                cpu_state.seg.ss = value;
                reg_keys.push(key.clone());
            }
            "xmm0" | "xmm1" | "xmm2" | "xmm3" | "xmm4" | "xmm5" | "xmm6" | "xmm7" | "xmm8"
            | "xmm9" | "xmm10" | "xmm11" | "xmm12" | "xmm13" | "xmm14" | "xmm15" => {
                let idx = key[3..].parse::<usize>()?;
                let mut bytes = [0; 16];
                set_vector(&mut bytes, value);
                cpu_state.avx.set_xmm(idx, &bytes);
                reg_keys.push(key.clone());
            }
            "ymm0" | "ymm1" | "ymm2" | "ymm3" | "ymm4" | "ymm5" | "ymm6" | "ymm7" | "ymm8"
            | "ymm9" | "ymm10" | "ymm11" | "ymm12" | "ymm13" | "ymm14" | "ymm15" => {
                let idx = key[3..].parse::<usize>()?;
                let mut bytes = [0; 32];
                set_vector(&mut bytes, value);
                cpu_state.avx.set_ymm(idx, &bytes);
                reg_keys.push(key.clone());
            }
            "zmm0" | "zmm1" | "zmm2" | "zmm3" => {
                let idx = key[3..].parse::<usize>()?;
                let mut bytes = [0; 64];
                set_vector(&mut bytes, value);
                cpu_state.avx.set_zmm(idx, &bytes);
                reg_keys.push(key.clone());
            }
            other => return Err(format!("unknown CPU state key: {other}").into()),
        }
    }

    Ok(reg_keys)
}

fn get_state_value(cpu_state: &CpuState, key: &str) -> Result<Value, Box<dyn Error>> {
    if let Some(index) = x87_logical_register_index(key) {
        return Ok(json_x87(cpu_state.fpu.st(index)));
    }
    if let Some(index) = x87_physical_register_index(key) {
        return Ok(json_x87(&cpu_state.fpu.registers[index]));
    }

    let value = match key {
        "x87_control" => cpu_state.fpu.control as u64,
        "x87_status" => cpu_state.fpu.status as u64,
        "x87_top" => cpu_state.fpu.top() as u64,
        "x87_tag" => cpu_state.fpu.tag as u64,
        "x87_opcode" => cpu_state.fpu.opcode as u64,
        "x87_ip" => cpu_state.fpu.instruction_pointer,
        "x87_dp" => cpu_state.fpu.data_pointer,
        "scratch_memory" => return Ok(json_scratch_memory(&cpu_state.scratch_memory)),
        "mem0_value" => cpu_state.mem0,
        "mem1_value" => cpu_state.mem1,
        "mm0" => cpu_state.fpu.mmx(0),
        "mm1" => cpu_state.fpu.mmx(1),
        "mm2" => cpu_state.fpu.mmx(2),
        "mm3" => cpu_state.fpu.mmx(3),
        "mm4" => cpu_state.fpu.mmx(4),
        "mm5" => cpu_state.fpu.mmx(5),
        "mm6" => cpu_state.fpu.mmx(6),
        "mm7" => cpu_state.fpu.mmx(7),
        "rax" => cpu_state.gpr.rax,
        "rbx" => cpu_state.gpr.rbx,
        "rcx" => cpu_state.gpr.rcx,
        "rdx" => cpu_state.gpr.rdx,
        "rsi" => cpu_state.gpr.rsi,
        "rdi" => cpu_state.gpr.rdi,
        "rbp" => cpu_state.gpr.rbp,
        "rsp" => cpu_state.gpr.rsp,
        "r8" => cpu_state.gpr.r8,
        "r9" => cpu_state.gpr.r9,
        "r10" => cpu_state.gpr.r10,
        "r11" => cpu_state.gpr.r11,
        "r12" => cpu_state.gpr.r12,
        "r13" => cpu_state.gpr.r13,
        "r14" => cpu_state.gpr.r14,
        "r15" => cpu_state.gpr.r15,
        "rip" => cpu_state.rip,
        "cs" => cpu_state.seg.cs,
        "ds" => cpu_state.seg.ds,
        "es" => cpu_state.seg.es,
        "fs" => cpu_state.seg.fs,
        "gs" => cpu_state.seg.gs,
        "ss" => cpu_state.seg.ss,
        "xmm0" | "xmm1" | "xmm2" | "xmm3" | "xmm4" | "xmm5" | "xmm6" | "xmm7" | "xmm8" | "xmm9"
        | "xmm10" | "xmm11" | "xmm12" | "xmm13" | "xmm14" | "xmm15" => {
            let idx = key[3..].parse::<usize>()?;
            u64::from_le_bytes(cpu_state.avx.get_xmm(idx)[..8].try_into()?)
        }
        "ymm0" | "ymm1" | "ymm2" | "ymm3" | "ymm4" | "ymm5" | "ymm6" | "ymm7" | "ymm8" | "ymm9"
        | "ymm10" | "ymm11" | "ymm12" | "ymm13" | "ymm14" | "ymm15" => {
            let idx = key[3..].parse::<usize>()?;
            u64::from_le_bytes(cpu_state.avx.get_ymm(idx)[..8].try_into()?)
        }
        "zmm0" | "zmm1" | "zmm2" | "zmm3" => {
            let idx = key[3..].parse::<usize>()?;
            u64::from_le_bytes(cpu_state.avx.get_zmm(idx)[..8].try_into()?)
        }
        other => return Err(format!("unknown CPU state key: {other}").into()),
    };
    Ok(json_u64(value))
}

fn serialize_state(
    cpu_state: &CpuState,
    reg_keys: &[String],
    include_rdx: bool,
) -> Result<Value, Box<dyn Error>> {
    let mut keys = Vec::new();
    for key in reg_keys {
        if !keys.contains(key) {
            keys.push(key.clone());
        }
    }
    if include_rdx && !keys.iter().any(|key| key == "rdx") {
        keys.push("rdx".to_string());
    }

    let mut state = Map::new();
    for key in keys {
        state.insert(key.clone(), get_state_value(cpu_state, &key)?);
    }
    state.insert("flag".to_string(), json_u64(cpu_state.flags.0));
    Ok(Value::Object(state))
}

fn flush_results(state: &mut RunnerState) -> Result<(), Box<dyn Error>> {
    if state.pending.is_empty() {
        return Ok(());
    }

    let pending = std::mem::take(&mut state.pending);
    let mut tc_ids = Vec::with_capacity(pending.len());
    let mut state_idxs = Vec::with_capacity(pending.len());
    let mut exceptions = Vec::with_capacity(pending.len());
    let mut finals: Vec<Option<Json<Value>>> = Vec::with_capacity(pending.len());

    for r in pending {
        tc_ids.push(r.test_case_id);
        state_idxs.push(r.state_index);
        exceptions.push(r.exception_kind);
        finals.push(r.final_state.map(Json));
    }

    state.db.execute(
        "
        INSERT INTO test_results (test_case_id, state_index, exception_kind, final_state)
        SELECT * FROM unnest($1::bigint[], $2::int[], $3::text[], $4::jsonb[])
        ON CONFLICT (test_case_id, state_index) DO UPDATE SET
            exception_kind = EXCLUDED.exception_kind,
            final_state    = EXCLUDED.final_state
        ",
        &[&tc_ids, &state_idxs, &exceptions, &finals],
    )?;

    Ok(())
}

fn writer(
    fetch: &Arc<Mutex<FetchState>>,
    runner: &Arc<Mutex<RunnerState>>,
    id: usize,
) -> Option<NamedTestCase> {
    // Fetch and CPU-state setup happen under the fetch lock, not the runner lock.
    // This means flush_results (reader) is never blocked by DB fetch (writer).
    let (spec, cpu_state, reg_keys) = {
        let mut fetch = fetch.lock().expect("fetch state mutex was poisoned");
        fetch_next_specs(&mut fetch).expect("failed to fetch test cases from database");
        let spec = fetch.buffered_specs.pop_front()?;

        let mut cpu_state = CpuState::zero();
        let reg_keys = apply_state(&mut cpu_state, &spec.initial_state)
            .expect("failed to apply initial CPU state");
        (spec, cpu_state, reg_keys)
    };

    let mut insn = [0u8; 15];
    if spec.encoding.len() > insn.len() {
        panic!(
            "test case {} has instruction longer than 15 bytes: {}",
            spec.test_case_id,
            spec.encoding.len()
        );
    }
    insn[..spec.encoding.len()].copy_from_slice(&spec.encoding);

    runner
        .lock()
        .expect("runner state mutex was poisoned")
        .active_tests
        .insert(
            id,
            ActiveTest {
                test_case_id: spec.test_case_id,
                state_index: spec.state_index,
                reg_keys,
            },
        );

    Some(NamedTestCase {
        test_case: TestCase {
            id,
            insn,
            size: spec.encoding.len() as u8,
            state: cpu_state,
        },
    })
}

fn reader(runner: &Arc<Mutex<RunnerState>>, result: Test) {
    let mut state = runner.lock().expect("runner state mutex was poisoned");
    let Some(active) = state.active_tests.remove(&result.id) else {
        return;
    };

    let pending = if let Some(end_state) = result.end_state {
        PendingResult {
            test_case_id: active.test_case_id,
            state_index: active.state_index,
            exception_kind: None,
            final_state: Some(
                serialize_state(&end_state, &active.reg_keys, true)
                    .expect("failed to serialize final CPU state"),
            ),
        }
    } else {
        PendingResult {
            test_case_id: active.test_case_id,
            state_index: active.state_index,
            exception_kind: result.exception_kind.map(|kind| kind.to_string()),
            final_state: None,
        }
    };

    state.pending.push(pending);
    state.progress.inc(1);

    if state.pending.len() >= state.batch_size {
        flush_results(&mut state).expect("failed to flush test results");
    }
}

fn build_progress() -> ProgressBar {
    let progress = ProgressBar::new_spinner();
    let style =
        ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] {pos} done ({per_sec})")
            .unwrap();
    progress.set_style(style);
    progress
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let mut db = PgClient::connect(&args.dsn, NoTls)?;
    let completed = if args.ignore_completed {
        HashSet::new()
    } else {
        fetch_completed(&mut db)?
    };
    let fetch_db = PgClient::connect(&args.dsn, NoTls)?;

    let fetch = Arc::new(Mutex::new(FetchState {
        db: fetch_db,
        completed,
        buffered_specs: VecDeque::new(),
        last_test_case_id: 0,
        exhausted: false,
        fetch_cases: args.fetch_cases,
        test_case_ids: args.test_case_ids,
    }));

    let runner = Arc::new(Mutex::new(RunnerState {
        db,
        active_tests: HashMap::new(),
        pending: Vec::new(),
        progress: build_progress(),
        batch_size: args.batch_size,
    }));

    let mut aegis = Aegis::new(&args.serial_sock, &args.shared_mem);

    let reader_runner = Arc::clone(&runner);
    aegis.set_read_executor(move |result| reader(&reader_runner, result));

    let writer_fetch = Arc::clone(&fetch);
    let writer_runner = Arc::clone(&runner);
    aegis.set_write_executor(move |id| writer(&writer_fetch, &writer_runner, id));

    aegis.init();
    aegis.run();

    let mut state = runner.lock().expect("runner state mutex was poisoned");
    flush_results(&mut state)?;
    state.progress.finish();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f80(byte: u8) -> Value {
        Value::String(format!("{byte:02x}").repeat(10))
    }

    #[test]
    fn physical_x87_json_round_trips_without_top_mapping() {
        let mut state = Map::new();
        state.insert("x87_status".into(), Value::from(3 << 11));
        state.insert("x87_top".into(), Value::from(3));
        state.insert("x87_tag".into(), Value::from(0x24));
        state.insert("x87_r0".into(), f80(0x10));
        state.insert("x87_r3".into(), f80(0x33));

        let mut cpu = CpuState::zero();
        let keys = apply_state(&mut cpu, &state).unwrap();

        assert_eq!(cpu.fpu.registers[0], [0x10; 10]);
        assert_eq!(cpu.fpu.registers[3], [0x33; 10]);
        assert_eq!(cpu.fpu.top(), 3);
        assert_eq!(cpu.fpu.tag, 0x24);
        assert_eq!(get_state_value(&cpu, "x87_r0").unwrap(), f80(0x10));
        assert_eq!(get_state_value(&cpu, "x87_top").unwrap(), Value::from(3));
        assert_eq!(get_state_value(&cpu, "x87_tag").unwrap(), Value::from(0x24));
        assert!(keys.contains(&"x87_r0".to_string()));
        assert!(!keys.contains(&"x87_top".to_string()));
        let serialized = serialize_state(&cpu, &keys, false).unwrap();
        assert!(serialized.get("x87_top").is_none());
        assert_eq!(serialized["x87_status"], Value::from(3 << 11));
    }

    #[test]
    fn physical_x87_and_mmx_overlap_must_agree() {
        let mut state = Map::new();
        state.insert("x87_r2".into(), f80(0x11));
        state.insert("mm2".into(), Value::from(0x2222u64));

        let error = apply_state(&mut CpuState::zero(), &state).unwrap_err();
        assert!(error.to_string().contains("mm2 conflicts with x87_r2"));
    }

    #[test]
    fn scratch_memory_round_trips_and_excludes_legacy_words() {
        let bytes: Vec<u8> = (0..SCRATCH_MEMORY_SIZE).map(|index| index as u8).collect();
        let mut state = Map::new();
        state.insert(
            "scratch_memory".into(),
            Value::String(bytes.iter().map(|byte| format!("{byte:02x}")).collect()),
        );

        let mut cpu = CpuState::zero();
        let keys = apply_state(&mut cpu, &state).unwrap();
        assert_eq!(cpu.scratch_memory.as_slice(), bytes);
        assert_eq!(cpu.scratch_memory_len as usize, SCRATCH_MEMORY_SIZE);
        assert_eq!(
            get_state_value(&cpu, "scratch_memory").unwrap(),
            state["scratch_memory"]
        );
        assert!(keys.contains(&"scratch_memory".to_string()));

        state.insert("mem0_value".into(), Value::from(0));
        let error = apply_state(&mut CpuState::zero(), &state).unwrap_err();
        assert!(error.to_string().contains("scratch_memory cannot be mixed"));
    }
}
