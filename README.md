# Aegis x86_64

Aegis executes x86-64 instruction test cases inside a small QEMU/KVM kernel and
stores the observed CPU state in a DB.
It is intended for validating IR lifters against real hardware behavior.

## Architecture

Aegis has three Rust crates:

- `aegis`: the `no_std` kernel that runs inside QEMU.
- `client`: the host executable that reads test cases from PostgreSQL, sends
  them to the VM, and stores results.
- `libaegis`: shared protocol and CPU-state definitions used by both sides.

The host and VM exchange bulk data through QEMU ivshmem and coordinate
transfers through a Unix serial socket.

## Requirements

- Linux with QEMU, KVM support.
- A CPU with SSE, AVX, AVX-512F, XSAVE, and OSXSAVE support.
- Rust nightly as pinned by `rust-toolchain.toml`.
- The `rust-src` component and the `bootimage` and `just` commands.
- A prepared PostgreSQL x86db database containing test cases and accepting
  result writes.

Install the Rust tooling with:

```bash
rustup component add rust-src
cargo install bootimage
cargo install just
```

## Running

Run the kernel and host client together:

```bash
just run
```

The default database connection is:

```text
postgresql://x86db:x86db@localhost:5432/x86db
```

Override it with `X86DB_DSN`:

```bash
X86DB_DSN=postgresql://user:password@host/database just run
```

Pass client arguments through the dedicated recipe:

```bash
just client --help
just client --ignore-completed
```

## Validation

Run formatting checks, component-specific builds, strict Clippy checks, and
unit tests:

```bash
just validate
```

## Known Limitation

The CPU-state protocol still exposes XMM, YMM, and ZMM values, but the kernel's
XSAVE/XRSTOR instructions are currently disabled.

Sometimes their is a race condition with the initial "HELLO" message.
Restarting usually does the trick.

## Resources
https://os.phil-opp.com/