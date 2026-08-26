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

## x87 state protocol

x87 states use `x87_r0` through `x87_r7`, each a 20-digit, little-endian
hexadecimal string holding an exact 80-bit physical FPU register R0–R7. TOP is
recorded only in bits 11–13 of numeric `x87_status`; consumers derive logical
`ST(i)` as `R[((x87_status >> 11) + i) & 7]`. A redundant `x87_top` input is
accepted only for validation/legacy fixtures and is never serialized in a
result. The remaining numeric fields are `x87_control`, `x87_tag`,
`x87_opcode`, `x87_ip`, and `x87_dp`. `x87_tag` is FXSAVE's physical-order
abridged tag byte (a set bit is non-empty).

`mm0`–`mm7` are low-64 views of physical R0–R7. A row may provide both views,
but values that overlap must agree. MMX-only rows retain the historical
all-active MMX initialization; mixed rows preserve the supplied x87
control/tag state. Legacy `x87_st0`–`x87_st7` input remains accepted for
compatibility but cannot be mixed with physical `x87_rN` fields.

For environment-memory instructions, `scratch_memory` is an optional 1024-digit
lowercase hexadecimal string representing 512 raw bytes rooted at the `mem0`
address. It preserves FXSAVE/FXRSTOR payload bytes. It cannot be combined with
legacy `mem0_value` or `mem1_value` fields; those remain 64-bit word views for
ordinary memory tests.

## Known Limitation

The kernel restores and captures the legacy FXSAVE state (x87/MMX and XMM)
around every test. XSAVE/XRSTOR remains disabled, so YMM/ZMM upper halves are
not yet restored or captured.

Sometimes their is a race condition with the initial "HELLO" message.
Restarting usually does the trick.

## Resources
https://os.phil-opp.com/