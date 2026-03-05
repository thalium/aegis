use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{BufRead, BufReader, ErrorKind, Write},
    os::unix::net::UnixStream,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, sleep, JoinHandle},
    time::{self, Duration, Instant},
};

use futures::{channel::mpsc, executor::block_on, stream::StreamExt, SinkExt};

use iced_x86::{Encoder, Instruction};
use libaegis::{
    cpu::CpuState,
    protocol::{CONTINUE_MSG, EXIT_MSG, INIT_MSG, READ_MSG, WRITE_MSG, WRITE_REGION_OFFSET},
    testcase::{ExceptionKind, TestCase, TestId, TestOutcome, TestResult},
};
use memmap2::MmapMut;

pub mod generator;

const TX_BUFFER_SIZE: usize = 256;

#[derive(Clone)]
pub struct Test {
    pub id: usize,
    pub name: String,
    pub instruction: Vec<u8>,
    pub start_state: CpuState,
    pub end_state: Option<CpuState>,
    pub exception_kind: Option<ExceptionKind>,
    pub exception_instruction: Vec<u8>,
}

#[derive(Clone)]
pub struct NamedTestCase {
    pub name: String,
    pub test_case: TestCase,
}

struct Reader<R> {
    rx: mpsc::Receiver<Test>,
    executor: R,
}

impl<R> Reader<R>
where
    R: FnMut(Test),
{
    async fn run(mut self) {
        while let Some(test) = self.rx.next().await {
            (self.executor)(test);
        }
    }
}

struct Writer<W> {
    tx: mpsc::Sender<NamedTestCase>,
    executor: W,
}

impl<W> Writer<W>
where
    W: FnMut(usize) -> Option<NamedTestCase>,
{
    async fn run(mut self) {
        let mut id = 1;

        while let Some(test) = (self.executor)(id) {
            id += 1;
            self.tx.send(test).await.expect("Error sending testcase");
        }
    }
}

struct PeekableReciever<T> {
    next: Option<T>,
    rx: mpsc::Receiver<T>,
}

impl<T> PeekableReciever<T> {
    fn new(rx: mpsc::Receiver<T>) -> Self {
        Self { rx, next: None }
    }

    async fn peek(&mut self) -> Option<&T> {
        if self.next.is_none() {
            self.next = self.rx.next().await;
        }

        self.next.as_ref()
    }

    async fn next(&mut self) -> Option<T> {
        match self.next.take() {
            Some(v) => Some(v),
            None => self.rx.next().await,
        }
    }
}

#[derive(Copy, Clone, PartialEq, PartialOrd)]
pub enum Verbosity {
    Quiet = 0,
    Normal = 1,
    Verbose = 2,
}

macro_rules! log {
    ($self:expr, $level:expr, $($arg:tt)*) => {{
        $self.log($level, format_args!($($arg)*));
    }};
}

/// Creates a test case from a icec-x86 instruction
pub fn make_test_case(
    zero_state: &CpuState,
    id: usize,
    insn: Instruction,
    state: CpuState,
) -> Result<TestCase, ()> {
    let mut encoder = Encoder::new(64);
    encoder.encode(&insn, zero_state.rip).map_err(|_| ())?;
    let mut buff = encoder.take_buffer();
    let size = buff.len() as u8;
    buff.resize(15, 0);

    Ok(TestCase {
        id,
        state,
        size,
        insn: buff.as_slice().try_into().unwrap(),
    })
}

pub struct AegisConfig<'a> {
    pub serial_sock: &'a str,
    pub shared_mem: &'a str,
    pub verbosity: Verbosity,
    pub cancel: Arc<AtomicBool>,
}

impl<'a> Default for AegisConfig<'a> {
    fn default() -> Self {
        Self {
            serial_sock: "",
            shared_mem: "",
            verbosity: Verbosity::Normal,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub struct Aegis {
    sock: UnixStream,
    mmap: MmapMut,

    verbosity: Verbosity,

    rx: Option<PeekableReciever<NamedTestCase>>,
    writer_handle: Option<JoinHandle<()>>,

    tx: Option<mpsc::Sender<Test>>,
    reader_handle: Option<JoinHandle<()>>,

    /// The "zero state" of the CPU
    pub zero_state: CpuState,

    states: HashMap<TestId, NamedTestCase>,

    cancel: Arc<AtomicBool>,
}

impl Aegis {
    pub fn new(config: AegisConfig) -> Self {
        // Wait for the shared_mem and serial_sock files to exist
        loop {
            if config.cancel.load(Ordering::Relaxed) {
                panic!("Interrupted");
            }

            if Path::new(config.serial_sock).exists() && Path::new(config.shared_mem).exists() {
                break;
            }

            sleep(time::Duration::from_millis(100));
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(config.shared_mem)
            .expect("Cannot open file for shared memory");

        // SAFETY: the file descriptor remains open for the lifetime of the mmap
        let mmap = unsafe { MmapMut::map_mut(&file).expect("Could not mmap") };

        let sock = UnixStream::connect(config.serial_sock).expect("Failed to open socket");
        sock.set_read_timeout(Some(Duration::from_millis(100)))
            .expect("Cannot set read timeout");

        Self {
            sock,
            mmap,
            rx: None,
            tx: None,
            reader_handle: None,
            writer_handle: None,
            verbosity: config.verbosity,
            cancel: config.cancel,
            zero_state: CpuState::zero(),
            states: HashMap::new(),
        }
    }

    fn maybe_exit(&self) {
        if self.cancel.load(Ordering::Relaxed) {
            panic!("Interrupted")
        }
    }

    pub fn set_read_executor<R>(&mut self, read_executor: R)
    where
        R: FnMut(Test) + Send + 'static,
    {
        if self.reader_handle.is_some() {
            panic!("The read executor was already set");
        }

        let (tx, rx) = mpsc::channel::<Test>(TX_BUFFER_SIZE);

        let reader = Reader {
            rx,
            executor: read_executor,
        };

        // Spawn a thread for the reader
        let reader_handle = thread::spawn(move || {
            block_on(reader.run());
        });

        self.tx = Some(tx);
        self.reader_handle = Some(reader_handle);
    }

    pub fn set_write_executor<W>(&mut self, write_executor: W)
    where
        W: FnMut(usize) -> Option<NamedTestCase> + Send + 'static,
    {
        if self.writer_handle.is_some() {
            panic!("The write executor was already set");
        }

        let (tx, rx) = mpsc::channel::<NamedTestCase>(TX_BUFFER_SIZE);

        let writer = Writer {
            tx,
            executor: write_executor,
        };

        // Spawn a thread for the reader
        let writer_handle = thread::spawn(move || {
            block_on(writer.run());
        });

        self.rx = Some(PeekableReciever::new(rx));
        self.writer_handle = Some(writer_handle);
    }

    fn log(&self, level: Verbosity, args: std::fmt::Arguments) {
        if level <= self.verbosity {
            println!("{}", args);
        }
    }

    /// Recieves a single line from the unix socket
    fn recv_line(&mut self) -> String {
        let mut reader = BufReader::new(&self.sock);
        let mut line = String::new();

        loop {
            match reader.read_line(&mut line) {
                Ok(_) => break,
                Err(e) => match e.kind() {
                    ErrorKind::WouldBlock | ErrorKind::TimedOut => (),
                    _ => panic!("Error while reading {}", e),
                },
            }

            self.maybe_exit();
        }

        line.trim().to_owned()
    }

    /// Gets the read buffer from a "read" serial command
    fn read_get_buff<'a>(&'a self, args: &str) -> &'a [u8] {
        let size: usize = args.parse().unwrap();
        log!(
            self,
            Verbosity::Verbose,
            "[*] Received read instruction: {} bytes",
            size
        );

        self.mmap.flush().unwrap();
        &self.mmap[WRITE_REGION_OFFSET..WRITE_REGION_OFFSET + size]
    }

    /// Gets the test cases from a "read" serial command
    fn read_get_tests(&mut self, args: &str) -> Vec<TestResult> {
        let mut buff = self.read_get_buff(args);

        let mut results = vec![];

        while !buff.is_empty() {
            let (b, result) = TestResult::from_bytes(buff).expect("Failed to read");
            buff = b;
            results.push(result);
        }

        writeln!(self.sock, "{}", CONTINUE_MSG).expect("[*] Failed to send continue message");

        results
    }

    /// Handles the "read" command
    async fn handle_read(&mut self, args: &str) {
        for result in self.read_get_tests(args) {
            let previous = self.states.remove(&result.id).unwrap();

            let name = previous.name;
            let previous = previous.test_case;

            self.tx
                .as_mut()
                .expect("the writer was not initialized")
                .send(Test {
                    id: result.id,
                    name,
                    instruction: previous
                        .insn
                        .iter()
                        .take(previous.size as usize)
                        .copied()
                        .collect::<Vec<u8>>(),
                    start_state: previous.state.clone(),
                    end_state: match &result.outcome {
                        TestOutcome::Completed(diff) => Some(previous.state.diff(diff)),
                        TestOutcome::Exception(_) => None,
                    },
                    exception_kind: match &result.outcome {
                        TestOutcome::Completed(_) => None,
                        TestOutcome::Exception(info) => Some(info.kind),
                    },
                    exception_instruction: match &result.outcome {
                        TestOutcome::Completed(_) => vec![],
                        TestOutcome::Exception(info) => info
                            .insn
                            .iter()
                            .take(info.size as usize)
                            .copied()
                            .collect::<Vec<u8>>(),
                    },
                })
                .await
                .expect("Error");
        }
    }

    /// Returns the write buffer (with lifetime removed)
    fn write_ptr(&mut self) -> &'static mut [u8] {
        unsafe { &mut *(&mut self.mmap[..WRITE_REGION_OFFSET] as *mut [u8]) }
    }

    /// Flush the write
    fn flush_write(&mut self, end_ptr: *const u8) {
        let start = self.write_ptr().as_ptr();

        if let Err(e) = self.mmap.flush() {
            eprintln!("Flush failed: {:?}", e);
        }

        let written_bytes = unsafe { end_ptr.offset_from(start) };
        writeln!(self.sock, "{}: {}", CONTINUE_MSG, written_bytes)
            .expect("Error sending continue message");

        log!(
            self,
            Verbosity::Verbose,
            "[*] Sent: {} bytes",
            written_bytes
        );
    }

    /// Handles the "write" command
    async fn handle_write(&mut self) {
        let mut buff = self.write_ptr();
        let mut end_ptr: *const u8;

        let rx = self.rx.as_mut().expect("Reader was not initialized");

        loop {
            end_ptr = buff.as_mut_ptr();

            let test = rx.peek().await;

            let test = match test {
                Some(test) => test,
                None => {
                    // Iterator is empty
                    break;
                }
            };

            self.states.insert(test.test_case.id, test.clone());

            let test = &test.test_case;

            match test.to_bytes(buff) {
                Ok(next_buff) => {
                    buff = next_buff;
                }
                Err(()) => {
                    // We filled the buffer
                    break;
                }
            }

            rx.next().await;
        }

        self.flush_write(end_ptr);
    }

    // Executes a nop instruction with a zero state to observe the "zero CPU" state (this gives us the avx header as well as rip and rsp)
    fn init_zero_state(&mut self) {
        let insn_len: u64;

        // Send a test with a zero state and a nop instruction
        {
            let line = self.recv_line();
            let (command, _) = Self::parse_line(&line);
            assert_eq!(command, WRITE_MSG);

            let zero_test = make_test_case(
                &self.zero_state,
                0,
                Instruction::with(iced_x86::Code::Nopw),
                CpuState::zero(),
            )
            .expect("The zero test should compile");
            insn_len = zero_test.size as u64;

            let ptr = self.write_ptr();
            let ptr = zero_test.to_bytes(ptr).unwrap();
            self.flush_write(ptr.as_ptr());
        }

        // Reads the zero state
        {
            let line = self.recv_line();
            let (command, args) = Self::parse_line(&line);
            assert_eq!(command, READ_MSG);

            let results = self.read_get_tests(args);
            assert_eq!(results.len(), 1);

            self.zero_state = match results.into_iter().next().unwrap().outcome {
                TestOutcome::Completed(state) => state,
                TestOutcome::Exception(exception) => {
                    panic!(
                        "Initialization test failed with exception {}",
                        exception.kind.as_str()
                    )
                }
            };
            self.zero_state.rip -= insn_len;
        }
    }

    /// Initializes client
    pub fn init(&mut self) {
        // Try to initialize the socket
        loop {
            writeln!(self.sock, "{}", INIT_MSG).expect("[*] Failed to send init message");

            let mut reader = BufReader::new(&self.sock);
            let mut line = String::new();

            match reader.read_line(&mut line) {
                Ok(_) => break,
                Err(e) => match e.kind() {
                    ErrorKind::WouldBlock | ErrorKind::TimedOut => (),
                    _ => panic!("Error while reading {}", e),
                },
            }

            if line == INIT_MSG {
                break;
            }

            sleep(Duration::from_millis(500));

            self.maybe_exit();
        }

        // Retrieve the zero state
        self.init_zero_state();
    }

    // Splits the command from the arguments
    fn parse_line(line: &str) -> (&str, &str) {
        match line.split_once(": ") {
            Some((command, args)) => (command, args),
            None => (line, ""),
        }
    }

    /// Main loop
    pub fn run(&mut self) {
        let start = Instant::now();

        loop {
            let line = self.recv_line();

            let (command, args) = Self::parse_line(&line);

            match command {
                READ_MSG => block_on(self.handle_read(args)),

                WRITE_MSG => block_on(self.handle_write()),

                EXIT_MSG => break,

                _ => panic!("[*] Unknown instruction: {}", line),
            }

            self.maybe_exit();
        }

        // Waits on the reader
        self.tx
            .as_mut()
            .expect("The writer was not initialized")
            .close_channel();
        self.reader_handle.take().map(|handle| handle.join());

        log!(
            self,
            Verbosity::Normal,
            "[*] Up time {:?}",
            Instant::now() - start
        );

        log!(self, Verbosity::Normal, "[*] Quitting...");
    }
}
