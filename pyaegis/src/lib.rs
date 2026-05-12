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

    use pyo3::{
        exceptions::{PyTypeError, PyValueError},
        prelude::*,
        types::{PyBytes, PyFunction, PyInt},
        PyAny, Python,
    };

    fn pyany_to_fixed_bytes<const N: usize>(
        value: &Bound<'_, PyAny>,
        reg: &str,
    ) -> PyResult<[u8; N]> {
        if let Ok(bytes) = value.extract::<Vec<u8>>() {
            return bytes
                .try_into()
                .map_err(|_| PyValueError::new_err(format!("{reg} must be exactly {N} bytes")));
        }

        if value.is_instance_of::<PyInt>() {
            let bytes = value
                .call_method1("to_bytes", (N, "little"))?
                .extract::<Vec<u8>>()?;

            return bytes.try_into().map_err(|_| {
                PyValueError::new_err(format!("{reg} conversion did not produce {N} bytes"))
            });
        }

        Err(PyTypeError::new_err(format!(
            "{reg} must be a Python int or bytes-like object"
        )))
    }

    fn fixed_bytes_to_pyint(py: Python<'_>, bytes: &[u8]) -> PyResult<Py<PyAny>> {
        let py_bytes = PyBytes::new(py, bytes);
        let value = py
            .get_type::<PyInt>()
            .call_method1("from_bytes", (py_bytes, "little"))?;
        Ok(value.unbind())
    }

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

        #[getter]
        fn mem0_value(&self) -> u64 {
            self.0.mem0
        }
        #[setter]
        fn set_mem0_value(&mut self, v: u64) {
            self.0.mem0 = v
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

        // XMM registers (128-bit / 16 bytes each, xmm0-xmm15)
        #[getter]
        fn xmm0(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_xmm(0))
        }
        #[setter]
        fn set_xmm0(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<16>(v, "xmm0")?;
            self.0.avx.set_xmm(0, &bytes);
            Ok(())
        }
        #[getter]
        fn xmm1(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_xmm(1))
        }
        #[setter]
        fn set_xmm1(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<16>(v, "xmm1")?;
            self.0.avx.set_xmm(1, &bytes);
            Ok(())
        }
        #[getter]
        fn xmm2(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_xmm(2))
        }
        #[setter]
        fn set_xmm2(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<16>(v, "xmm2")?;
            self.0.avx.set_xmm(2, &bytes);
            Ok(())
        }
        #[getter]
        fn xmm3(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_xmm(3))
        }
        #[setter]
        fn set_xmm3(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<16>(v, "xmm3")?;
            self.0.avx.set_xmm(3, &bytes);
            Ok(())
        }

        // YMM registers (256-bit / 32 bytes each, ymm0-ymm15)
        #[getter]
        fn ymm0(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(0))
        }
        #[setter]
        fn set_ymm0(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm0")?;
            self.0.avx.set_ymm(0, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm1(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(1))
        }
        #[setter]
        fn set_ymm1(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm1")?;
            self.0.avx.set_ymm(1, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm2(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(2))
        }
        #[setter]
        fn set_ymm2(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm2")?;
            self.0.avx.set_ymm(2, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm3(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(3))
        }
        #[setter]
        fn set_ymm3(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm3")?;
            self.0.avx.set_ymm(3, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm4(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(4))
        }
        #[setter]
        fn set_ymm4(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm4")?;
            self.0.avx.set_ymm(4, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm5(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(5))
        }
        #[setter]
        fn set_ymm5(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm5")?;
            self.0.avx.set_ymm(5, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm6(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(6))
        }
        #[setter]
        fn set_ymm6(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm6")?;
            self.0.avx.set_ymm(6, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm7(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(7))
        }
        #[setter]
        fn set_ymm7(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm7")?;
            self.0.avx.set_ymm(7, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm8(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(8))
        }
        #[setter]
        fn set_ymm8(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm8")?;
            self.0.avx.set_ymm(8, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm9(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(9))
        }
        #[setter]
        fn set_ymm9(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm9")?;
            self.0.avx.set_ymm(9, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm10(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(10))
        }
        #[setter]
        fn set_ymm10(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm10")?;
            self.0.avx.set_ymm(10, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm11(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(11))
        }
        #[setter]
        fn set_ymm11(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm11")?;
            self.0.avx.set_ymm(11, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm12(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(12))
        }
        #[setter]
        fn set_ymm12(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm12")?;
            self.0.avx.set_ymm(12, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm13(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(13))
        }
        #[setter]
        fn set_ymm13(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm13")?;
            self.0.avx.set_ymm(13, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm14(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(14))
        }
        #[setter]
        fn set_ymm14(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm14")?;
            self.0.avx.set_ymm(14, &bytes);
            Ok(())
        }
        #[getter]
        fn ymm15(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_ymm(15))
        }
        #[setter]
        fn set_ymm15(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<32>(v, "ymm15")?;
            self.0.avx.set_ymm(15, &bytes);
            Ok(())
        }

        // ZMM registers (512-bit / 64 bytes each, zmm0-zmm31)
        #[getter]
        fn zmm0(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_zmm(0))
        }
        #[setter]
        fn set_zmm0(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<64>(v, "zmm0")?;
            self.0.avx.set_zmm(0, &bytes);
            Ok(())
        }
        #[getter]
        fn zmm1(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_zmm(1))
        }
        #[setter]
        fn set_zmm1(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<64>(v, "zmm1")?;
            self.0.avx.set_zmm(1, &bytes);
            Ok(())
        }
        #[getter]
        fn zmm2(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_zmm(2))
        }
        #[setter]
        fn set_zmm2(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<64>(v, "zmm2")?;
            self.0.avx.set_zmm(2, &bytes);
            Ok(())
        }
        #[getter]
        fn zmm3(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
            fixed_bytes_to_pyint(py, &self.0.avx.get_zmm(3))
        }
        #[setter]
        fn set_zmm3(&mut self, v: &Bound<'_, PyAny>) -> PyResult<()> {
            let bytes = pyany_to_fixed_bytes::<64>(v, "zmm3")?;
            self.0.avx.set_zmm(3, &bytes);
            Ok(())
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
                    read.call1(py, (py_result,))
                        .inspect_err(|e| {
                            e.display(py);
                            panic!("Python error in read executor")
                        })
                        .unwrap();
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
                            if let Ok(test_case) = make_test_case(&zero_state, id, insn, state) {
                                let name = insn.to_string();
                                return Some(NamedTestCase { name, test_case });
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
