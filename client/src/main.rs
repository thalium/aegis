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
    cpu::{CpuState, FlagState},
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
        let rows = state.db.query(
            "
            SELECT id, opcode, initial_states
            FROM test_cases
            WHERE id > $1
            ORDER BY id
            LIMIT $2
            ",
            &[&state.last_test_case_id, &state.fetch_cases],
        )?;

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
    let mut reg_keys = Vec::new();

    for (key, value) in state_data {
        let value = value_to_u64(value)? & REG_VALUE_MASK;
        match key.as_str() {
            "flag" => cpu_state.flags = FlagState(value),
            "mem0_value" => {
                cpu_state.mem0 = value;
                reg_keys.push(key.clone());
            }
            "mm0" => {
                cpu_state.mmx.mm0 = value;
                reg_keys.push(key.clone());
            }
            "mm1" => {
                cpu_state.mmx.mm1 = value;
                reg_keys.push(key.clone());
            }
            "mm2" => {
                cpu_state.mmx.mm2 = value;
                reg_keys.push(key.clone());
            }
            "mm3" => {
                cpu_state.mmx.mm3 = value;
                reg_keys.push(key.clone());
            }
            "mm4" => {
                cpu_state.mmx.mm4 = value;
                reg_keys.push(key.clone());
            }
            "mm5" => {
                cpu_state.mmx.mm5 = value;
                reg_keys.push(key.clone());
            }
            "mm6" => {
                cpu_state.mmx.mm6 = value;
                reg_keys.push(key.clone());
            }
            "mm7" => {
                cpu_state.mmx.mm7 = value;
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

fn get_state_value(cpu_state: &CpuState, key: &str) -> Result<u64, Box<dyn Error>> {
    let value = match key {
        "mem0_value" => cpu_state.mem0,
        "mm0" => cpu_state.mmx.mm0,
        "mm1" => cpu_state.mmx.mm1,
        "mm2" => cpu_state.mmx.mm2,
        "mm3" => cpu_state.mmx.mm3,
        "mm4" => cpu_state.mmx.mm4,
        "mm5" => cpu_state.mmx.mm5,
        "mm6" => cpu_state.mmx.mm6,
        "mm7" => cpu_state.mmx.mm7,
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
    Ok(value & REG_VALUE_MASK)
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
        state.insert(key.clone(), json_u64(get_state_value(cpu_state, &key)?));
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
