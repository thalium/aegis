use std::env;
use std::fs::File;
use std::io::{self, Read};

use libaegis::compressible::Compressible;
use libaegis::cpu::CpuState;

use std::alloc::{Layout, alloc, dealloc};

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <file_path>", args[0]);
        std::process::exit(1);
    }
    let file_path = &args[1];

    let mut file = File::open(file_path)?;

    // Determine file size
    let file_size = file.metadata()?.len() as usize;

    // Create a 64-bit aligned buffer
    let layout = Layout::from_size_align(file_size, 8).expect("Invalid layout");
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        panic!("Failed to allocate memory");
    }

    // Create a slice from the raw pointer
    let buffer: &mut [u8] = unsafe { std::slice::from_raw_parts_mut(ptr, file_size) };

    // Read the file into the aligned buffer
    file.read_exact(buffer)?;

    println!(
        "Read {} bytes from {} with 64-bit alignment",
        buffer.len(),
        file_path
    );

    let mut rest: &[u8] = buffer;

    let mut state = CpuState::zero();
    let mut i = 0;
    while !rest.is_empty() {
        rest = CpuState::decompress(rest, &mut state).unwrap();
        println!("Found rax: {}", state.gpr.rax ^ i);
        println!(
            "Found other gpr: {:?}\nFlags: {}\n{:?}\n",
            state.gpr, state.flags, state
        );
        i += 1;
    }

    // Free the memory
    unsafe { dealloc(ptr, layout) };

    Ok(())
}
