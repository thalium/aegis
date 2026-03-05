#[pyo3::pymodule(gil_used = false)]
mod pyaegis {

    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread::{self, sleep},
        time::Duration,
    };

    use client::{generator, make_test_case, Aegis, AegisConfig, NamedTestCase, Test};
    use libaegis::{
        cpu::{CpuState, FlagState},
        protocol::{CONTINUE_MSG, EXIT_MSG, INIT_MSG, READ_MSG, WRITE_MSG},
        testcase::TestCase,
    };

    use pyo3::{prelude::*, types::PyFunction, PyAny, Python};

    #[pymodule_init]
    fn init(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add("CONTINUE_MSG", CONTINUE_MSG)?;
        m.add("EXIT_MSG", EXIT_MSG)?;
        m.add("INIT_MSG", INIT_MSG)?;
        m.add("READ_MSG", READ_MSG)?;
        m.add("WRITE_MSG", WRITE_MSG)?;
        Ok(())
    }

    #[pyclass]
    #[repr(transparent)]

    pub struct PyFlagState(FlagState);

    #[pymethods]
    impl PyFlagState {
        #[new]
        fn new(v: u64) -> Self {
            Self(FlagState(v))
        }

        fn __str__(&self) -> String {
            format!("{:?}", self.0)
        }

        fn compare(&self, other: &PyFlagState) -> String {
            FlagState::diff_flags(self.0, other.0)
        }

        #[getter]
        fn value(&self) -> u64 {
            self.0 .0
        }
    }

    #[pyclass]
    #[repr(transparent)]
    pub struct PyCpuState(CpuState);

    #[pymethods]
    impl PyCpuState {
        #[new]
        fn new() -> Self {
            Self(CpuState::zero())
        }

        fn __str__(&self) -> String {
            format!("{:?}", self.0)
        }

        // Methods
        /// Diffs this CPU state with another state
        fn diff(&self, other: &PyCpuState) -> PyCpuState {
            PyCpuState(self.0.diff(&other.0))
        }

        // MMX getters/setters
        #[getter]
        fn mm0(&self) -> u64 {
            self.0.mmx.mm0
        }
        #[setter]
        fn set_mm0(&mut self, v: u64) {
            self.0.mmx.mm0 = v
        }
        #[getter]
        fn mm1(&self) -> u64 {
            self.0.mmx.mm1
        }
        #[setter]
        fn set_mm1(&mut self, v: u64) {
            self.0.mmx.mm1 = v
        }
        #[getter]
        fn mm2(&self) -> u64 {
            self.0.mmx.mm2
        }
        #[setter]
        fn set_mm2(&mut self, v: u64) {
            self.0.mmx.mm2 = v
        }
        #[getter]
        fn mm3(&self) -> u64 {
            self.0.mmx.mm3
        }
        #[setter]
        fn set_mm3(&mut self, v: u64) {
            self.0.mmx.mm3 = v
        }
        #[getter]
        fn mm4(&self) -> u64 {
            self.0.mmx.mm4
        }
        #[setter]
        fn set_mm4(&mut self, v: u64) {
            self.0.mmx.mm4 = v
        }
        #[getter]
        fn mm5(&self) -> u64 {
            self.0.mmx.mm5
        }
        #[setter]
        fn set_mm5(&mut self, v: u64) {
            self.0.mmx.mm5 = v
        }
        #[getter]
        fn mm6(&self) -> u64 {
            self.0.mmx.mm6
        }
        #[setter]
        fn set_mm6(&mut self, v: u64) {
            self.0.mmx.mm6 = v
        }
        #[getter]
        fn mm7(&self) -> u64 {
            self.0.mmx.mm7
        }
        #[setter]
        fn set_mm7(&mut self, v: u64) {
            self.0.mmx.mm7 = v
        }

        #[getter]
        fn rax(&self) -> u64 {
            self.0.gpr.rax
        }
        #[setter]
        fn set_rax(&mut self, v: u64) {
            self.0.gpr.rax = v
        }
        #[getter]
        fn rbx(&self) -> u64 {
            self.0.gpr.rbx
        }
        #[setter]
        fn set_rbx(&mut self, v: u64) {
            self.0.gpr.rbx = v
        }
        #[getter]
        fn rcx(&self) -> u64 {
            self.0.gpr.rcx
        }
        #[setter]
        fn set_rcx(&mut self, v: u64) {
            self.0.gpr.rcx = v
        }
        #[getter]
        fn rdx(&self) -> u64 {
            self.0.gpr.rdx
        }
        #[setter]
        fn set_rdx(&mut self, v: u64) {
            self.0.gpr.rdx = v
        }
        #[getter]
        fn rsi(&self) -> u64 {
            self.0.gpr.rsi
        }
        #[setter]
        fn set_rsi(&mut self, v: u64) {
            self.0.gpr.rsi = v
        }
        #[getter]
        fn rdi(&self) -> u64 {
            self.0.gpr.rdi
        }
        #[setter]
        fn set_rdi(&mut self, v: u64) {
            self.0.gpr.rdi = v
        }
        #[getter]
        fn rbp(&self) -> u64 {
            self.0.gpr.rbp
        }
        #[setter]
        fn set_rbp(&mut self, v: u64) {
            self.0.gpr.rbp = v
        }
        #[getter]
        fn rsp(&self) -> u64 {
            self.0.gpr.rsp
        }
        #[setter]
        fn set_rsp(&mut self, v: u64) {
            self.0.gpr.rsp = v
        }
        #[getter]
        fn r8(&self) -> u64 {
            self.0.gpr.r8
        }
        #[setter]
        fn set_r8(&mut self, v: u64) {
            self.0.gpr.r8 = v
        }
        #[getter]
        fn r9(&self) -> u64 {
            self.0.gpr.r9
        }
        #[setter]
        fn set_r9(&mut self, v: u64) {
            self.0.gpr.r9 = v
        }
        #[getter]
        fn r10(&self) -> u64 {
            self.0.gpr.r10
        }
        #[setter]
        fn set_r10(&mut self, v: u64) {
            self.0.gpr.r10 = v
        }
        #[getter]
        fn r11(&self) -> u64 {
            self.0.gpr.r11
        }
        #[setter]
        fn set_r11(&mut self, v: u64) {
            self.0.gpr.r11 = v
        }
        #[getter]
        fn r12(&self) -> u64 {
            self.0.gpr.r12
        }
        #[setter]
        fn set_r12(&mut self, v: u64) {
            self.0.gpr.r12 = v
        }
        #[getter]
        fn r13(&self) -> u64 {
            self.0.gpr.r13
        }
        #[setter]
        fn set_r13(&mut self, v: u64) {
            self.0.gpr.r13 = v
        }
        #[getter]
        fn r14(&self) -> u64 {
            self.0.gpr.r14
        }
        #[setter]
        fn set_r14(&mut self, v: u64) {
            self.0.gpr.r14 = v
        }
        #[getter]
        fn r15(&self) -> u64 {
            self.0.gpr.r15
        }
        #[setter]
        fn set_r15(&mut self, v: u64) {
            self.0.gpr.r15 = v
        }

        // RIP
        #[getter]
        fn rip(&self) -> u64 {
            self.0.rip
        }
        #[setter]
        fn set_rip(&mut self, v: u64) {
            self.0.rip = v
        }

        // SEG
        #[getter]
        fn cs(&self) -> u64 {
            self.0.seg.cs
        }
        #[setter]
        fn set_cs(&mut self, v: u64) {
            self.0.seg.cs = v
        }
        #[getter]
        fn ds(&self) -> u64 {
            self.0.seg.ds
        }
        #[setter]
        fn set_ds(&mut self, v: u64) {
            self.0.seg.ds = v
        }
        #[getter]
        fn es(&self) -> u64 {
            self.0.seg.es
        }
        #[setter]
        fn set_es(&mut self, v: u64) {
            self.0.seg.es = v
        }
        #[getter]
        fn fs(&self) -> u64 {
            self.0.seg.fs
        }
        #[setter]
        fn set_fs(&mut self, v: u64) {
            self.0.seg.fs = v
        }
        #[getter]
        fn gs(&self) -> u64 {
            self.0.seg.gs
        }
        #[setter]
        fn set_gs(&mut self, v: u64) {
            self.0.seg.gs = v
        }
        #[getter]
        fn ss(&self) -> u64 {
            self.0.seg.ss
        }
        #[setter]
        fn set_ss(&mut self, v: u64) {
            self.0.seg.ss = v
        }

        // FLAGS
        #[getter]
        fn flags(&self) -> PyFlagState {
            PyFlagState(self.0.flags)
        }
        #[setter]
        fn set_flags(&mut self, v: u64) {
            self.0.flags.0 = v
        }
    }

    #[pyclass]
    #[repr(transparent)]
    pub struct PyTestCase(TestCase);

    #[pymethods]
    impl PyTestCase {
        #[new]
        pub fn new(id: usize, state: &PyCpuState, insn: &[u8]) -> Self {
            let size = insn.len();
            let mut arr = [0u8; 15];
            arr[..size].copy_from_slice(insn);

            Self(TestCase {
                id,
                state: state.0.clone(),
                size: size as u8,
                insn: arr,
            })
        }

        #[getter]
        pub fn id(&self) -> usize {
            self.0.id
        }

        #[getter]
        pub fn state(&self) -> PyCpuState {
            PyCpuState(self.0.state.clone())
        }

        #[getter]
        pub fn insn(&self) -> [u8; 15] {
            self.0.insn.into()
        }

        #[getter]
        pub fn size(&self) -> u8 {
            self.0.size
        }
    }

    #[pyclass]
    pub struct PyNamedTestCase {
        pub test_case: PyTestCase,
        pub name: String,
    }

    #[pymethods]
    impl PyNamedTestCase {
        #[new]
        pub fn new(id: usize, name: String, state: &PyCpuState, insn: &[u8]) -> Self {
            Self {
                test_case: PyTestCase::new(id, state, insn),
                name,
            }
        }
    }

    #[pyclass]
    #[repr(transparent)]
    pub struct PyTest(Test);

    #[pymethods]
    impl PyTest {
        #[new]
        pub fn new(
            id: usize,
            name: String,
            start_state: &PyCpuState,
            end_state: &PyCpuState,
            insn: &[u8],
        ) -> Self {
            Self(Test {
                id,
                name,
                start_state: start_state.0.clone(),
                end_state: Some(end_state.0.clone()),
                instruction: insn.to_owned(),
                exception_kind: None,
                exception_instruction: vec![],
            })
        }

        #[getter]
        pub fn id(&self) -> usize {
            self.0.id
        }

        #[getter]
        pub fn start_state(&self) -> PyCpuState {
            PyCpuState(self.0.start_state.clone())
        }

        #[getter]
        pub fn end_state(&self) -> Option<PyCpuState> {
            self.0.end_state.clone().map(PyCpuState)
        }

        #[getter]
        pub fn insn(&self) -> Vec<u8> {
            self.0.instruction.clone()
        }

        #[getter]
        pub fn exception_kind(&self) -> Option<String> {
            self.0
                .exception_kind
                .as_ref()
                .map(|kind| kind.as_str().to_owned())
        }

        #[getter]
        pub fn exception_insn(&self) -> Vec<u8> {
            self.0.exception_instruction.clone()
        }

        #[getter]
        pub fn name(&self) -> String {
            self.0.name.clone()
        }
    }

    #[pyclass]
    pub struct PyAegis {
        aegis: Option<Aegis>,
        cancel: Arc<AtomicBool>,
    }

    #[pymethods]
    impl PyAegis {
        #[new]
        #[pyo3(signature = (serial_sock, shared_mem, read, write, quiet = false))]
        pub fn new(
            py: Python<'_>,
            serial_sock: String,
            shared_mem: String,
            read: Py<PyFunction>,
            write: Bound<'_, PyAny>,
            quiet: bool,
        ) -> Self {
            Python::initialize();

            let cancel = Arc::new(AtomicBool::new(false));
            let cancel_aegis = cancel.clone();

            let handle = thread::spawn(move || {
                let mut aegis = Aegis::new(AegisConfig {
                    serial_sock: &serial_sock,
                    shared_mem: &shared_mem,
                    verbosity: if quiet {
                        client::Verbosity::Quiet
                    } else {
                        client::Verbosity::Verbose
                    },
                    cancel: cancel_aegis,
                });

                aegis.init();

                aegis
            });

            while !handle.is_finished() {
                py.detach(|| sleep(Duration::from_millis(100)));

                if py.check_signals().is_err() {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            let mut aegis = handle
                .join()
                .expect("An error occured during aegis initialization");

            aegis.set_read_executor(move |test: Test| {
                Python::attach(|py| {
                    let py_result = Py::new(py, PyTest(test)).unwrap();
                    read.call1(py, (py_result,)).unwrap();
                })
            });

            if let Ok(write) = write.extract::<Py<PyFunction>>() {
                aegis.set_write_executor(move |id: usize| -> Option<NamedTestCase> {
                    Python::try_attach(|py| {
                        // safe Python interaction here
                        let res = match write.call1(py, (id,)) {
                            Ok(r) => r,
                            Err(e) => {
                                e.print(py);
                                panic!("Python error");
                            }
                        };

                        let test: Option<Py<PyNamedTestCase>> = match res.extract(py) {
                            Ok(t) => t,
                            Err(_) => {
                                panic!("The genetator should return an optional PyNamedTestCase");
                            }
                        };

                        match test {
                            Some(test) => {
                                let test = test.borrow(py);
                                Some(NamedTestCase {
                                    name: test.name.clone(),
                                    test_case: test.test_case.0.clone(),
                                })
                            }
                            None => None,
                        }
                    })
                    .unwrap()
                });
            } else if let Ok(value) = write.extract::<String>() {
                let zero_state = aegis.zero_state.clone();
                match value.as_str() {
                    "random_8086" => aegis.set_write_executor(move |id: usize| {
                        let mut rng = rand::rng();

                        loop {
                            let insn = generator::random_insn(&mut rng);
                            let state = generator::random_state(&mut rng, zero_state.clone());

                            if id > 1000 {
                                return None;
                            }
                            match make_test_case(&zero_state, id, insn, state) {
                                Ok(test_case) => {
                                    let name = insn.to_string();
                                    return Some(NamedTestCase { name, test_case });
                                }
                                Err(()) => (),
                            }
                        }
                    }),
                    _ => panic!("Unknown string"),
                }
            } else {
                panic!("Unrecognized write argument");
            };

            Box::leak("a".to_string().into_boxed_str());

            Self {
                aegis: Some(aegis),
                cancel,
            }
        }

        pub fn run(&mut self, py: Python<'_>) {
            py.detach(|| sleep(Duration::from_millis(100)));

            let mut aegis = self.aegis.take().expect("Run can only be called once");

            let handle = thread::spawn(move || {
                aegis.run();
            });

            while !handle.is_finished() {
                py.detach(|| sleep(Duration::from_millis(100)));

                if py.check_signals().is_err() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
            }
            handle.join().expect("Error while running thread")
        }
    }
}
