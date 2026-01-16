use alloc::vec::Vec;
use libaegis::protocol::{CONTINUE_MSG, EXIT_MSG, INIT_MSG, READ_MSG, WRITE_MSG};
use spin::Mutex;
use x86_64::VirtAddr;

use crate::{
    kernel::{driver::serial::SERIAL1, qemu},
    println, serial_println,
};

pub static SHARED_MEMORY_MANAGER: Mutex<SharedMemoryManager> =
    Mutex::new(SharedMemoryManager::new());

/// Recieves bytes from a serial port until an end byte is recieved
fn recv_until(buff: &mut Vec<u8>, end: u8) {
    loop {
        let b = SERIAL1.lock().receive();

        if b == end {
            break;
        }

        // HACK: New line encoding
        if b == b'\r' {
            continue;
        }

        buff.push(b);
    }
}

/// Recieves a line from a sertial connection
fn recv_line(buff: &mut Vec<u8>) -> &str {
    recv_until(buff, b'\n');
    str::from_utf8(buff).expect("Recieved raw bytes")
}

#[derive(Debug)]
pub struct Region {
    start: VirtAddr,
    size: usize,
}

impl Region {
    /// Creates a new region
    pub const fn new(start: VirtAddr, size: usize) -> Self {
        Self { start, size }
    }

    /// Creates a null region
    const fn zero() -> Self {
        Region::new(VirtAddr::zero(), 0)
    }

    /// The end address of this region
    pub fn end(&self) -> VirtAddr {
        self.start + self.size
    }

    /// is this region empty
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
}

/// We use 2 stacks in ivshmem
/// - A readable stack, starting from the beginning of the shared memory
/// - A writable stack, starting halfway in the shared memory
///
///  read_start_ptr -> +------------------+
///                    |                  |
///                    |       READ       |
///                    |                  |
///    read_end_ptr -> +------------------+
///                    |             ...  |
///                    | ...              |
/// write_start_ptr -> +------------------+
///                    |                  |
///                    |      WRITE       |
///                    |                  |
///   write_end_ptr -> +------------------+
#[derive(Debug)]
pub struct SharedMemoryManager {
    /// A pointer to the readable data
    read_ptr: VirtAddr,

    /// A pointer to the writable data
    write_ptr: VirtAddr,

    /// The readable region
    read_mem: Region,

    /// The writable region
    write_mem: Region,
}

impl SharedMemoryManager {
    /// Creates a blank manager
    /// The manager needs to be initialized before it is used
    const fn new() -> Self {
        Self {
            read_ptr: VirtAddr::zero(),
            write_ptr: VirtAddr::zero(),
            read_mem: Region::zero(),
            write_mem: Region::zero(),
        }
    }

    /// Initializes the connection
    /// The client starts by sending "Hello\n"
    /// And we respond by saying "Hello\n"
    fn init_connection(buff: &mut Vec<u8>) {
        println!("[*] Awaiting client");

        loop {
            let res = recv_line(buff);
            println!("[*] Recieved client '{}' (expected '{}')", res, INIT_MSG);

            if res == INIT_MSG {
                break;
            }

            buff.clear();
        }

        serial_println!("{}", INIT_MSG);
        println!("[*] Sent '{}'", INIT_MSG);
    }

    /// Initializes the shared memroy
    pub fn init(&mut self, shared_memory: Region, write_mem_offset: usize) {
        self.read_mem = Region::new(shared_memory.start, 0);
        self.read_ptr = self.read_mem.start;

        self.write_mem = Region::new(
            shared_memory.start + write_mem_offset,
            shared_memory.size - write_mem_offset,
        );
        self.write_ptr = self.write_mem.start;

        let mut buff = Vec::new();
        Self::init_connection(&mut buff);
    }

    /// Writes `count` bytes into `dst`
    /// If not enough bytes are available, asks the client for more space
    pub fn write(&mut self, src: *const u8, count: usize) {
        // Is there enough room to write this data ?
        if self.write_ptr + count >= self.write_mem.end() {
            self.clear_write_buffer();
        }

        let dst = self.write_ptr.as_mut_ptr();

        unsafe {
            core::ptr::copy_nonoverlapping(src, dst, count);
        }

        self.write_ptr += count;
    }

    /// Clears the write buffer, requesting that the client read it before it is erased
    pub fn clear_write_buffer(&mut self) {
        // Request a read
        self.request_read();

        // Clear the memory
        self.write_ptr = self.write_mem.start;
    }

    /// Returns the memory region as a writable buffer
    pub fn write_buffer(&mut self) -> &'static mut [u8] {
        let start: *mut u8 = self.write_ptr.as_mut_ptr();
        let len = unsafe { self.write_mem.end().as_ptr::<u8>().offset_from(start) as usize };

        unsafe { core::slice::from_raw_parts_mut(start, len) }
    }

    /// Sets the writable buffer
    pub fn set_write_buffer(&mut self, buffer: &[u8]) {
        self.write_ptr = VirtAddr::new(buffer.as_ptr() as u64);
    }

    /// Refresh the read buffer, requesting that the client read it before it is erased
    pub fn refresh_read_buffer(&mut self) {
        // Request a read
        self.request_write();

        // Clear the memory
        self.read_ptr = self.read_mem.start;

        // Read the amount of bytes written
    }

    /// Returns the memory region as a writable buffer
    pub fn read_buffer(&mut self) -> &'static mut [u8] {
        let start: *mut u8 = self.read_ptr.as_mut_ptr();
        let len = unsafe { self.read_mem.end().as_ptr::<u8>().offset_from(start) as usize };

        unsafe { core::slice::from_raw_parts_mut(start, len) }
    }

    /// Sets the writable buffer
    pub fn set_read_buffer(&mut self, buffer: &[u8]) {
        self.read_ptr = VirtAddr::new(buffer.as_ptr() as u64);
    }

    /// Reads `count` bytes into `dst`
    /// If not enough bytes are available, asks the client for more data
    /// This could end the program
    pub fn read(&mut self, dst: *mut u8, count: usize) {
        // Is there enough room to write this data ?
        if self.read_ptr + count >= self.read_mem.end() {
            // Request a read
            self.request_write();

            // Clear the memory
            self.read_ptr = self.read_mem.start;
        }

        let src = self.write_ptr.as_ptr();

        unsafe {
            core::ptr::copy_nonoverlapping(src, dst, count);
        }

        self.read_ptr += count;
    }

    /// Requests the client to read the WRITE buffer so that we can continue writing to it
    fn request_read(&self) {
        let size = self.write_ptr - self.write_mem.start;

        println!("[*] Requesting read for {} bytes", size);
        serial_println!("{}: {}", READ_MSG, size);
        let mut buff = Vec::new();

        loop {
            match SERIAL1.lock().try_receive() {
                Ok(b) => {
                    buff.push(b);
                    break;
                }

                Err(_) => (),
            }
        }

        let line = recv_line(&mut buff);

        match line {
            CONTINUE_MSG => println!("[*] Done"),
            _ => panic!("Unexpected message: {}", line),
        }
    }

    /// Requests the client to write to the READ buffer so that we can continue reading it
    fn request_write(&mut self) {
        // We should be aligned
        assert!(self.read_mem.end() - self.read_ptr <= 1);

        println!("[*] Requesting write");
        serial_println!("{}", WRITE_MSG);
        let mut buff = Vec::new();

        loop {
            match SERIAL1.lock().try_receive() {
                Ok(b) => {
                    buff.push(b);
                    break;
                }

                Err(_) => (),
            }
        }

        let line = recv_line(&mut buff);

        if line.starts_with(CONTINUE_MSG) {
            let (_, size) = line.split_once(": ").expect("Client did not supply size");

            let size: usize = size.parse().expect("Unable to parse size");

            if size == 0 {
                self.exit();
            }

            println!("[*] Client wrote {:x}bytes", size);

            self.read_mem.size = size;
        } else if line == EXIT_MSG {
            self.exit();
        } else {
            panic!("Unexpected message: {}", line);
        }
    }

    /// Gracefully exits
    pub fn exit(&self) {
        // Ensure all memory gets saved
        if self.write_ptr != self.write_mem.start {
            self.request_read();
        }

        println!("[*] Exiting");

        serial_println!("{}", EXIT_MSG);
        qemu::exit_qemu(qemu::QemuExitCode::Success);
    }
}
