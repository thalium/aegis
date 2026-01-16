FROM rust:trixie AS builder

COPY . .
RUN rustc --version && cargo --version
WORKDIR "/aegis"
RUN rustup component add llvm-tools-preview rust-src --toolchain nightly-x86_64-unknown-linux-gnu
RUN cargo install bootimage
RUN cargo bootimage --release

FROM scratch
COPY --from=builder /target/x86_64-target/release/bootimage-aegis.bin /bootimage-aegis.bin