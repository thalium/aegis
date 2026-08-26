run *args:
    #!/usr/bin/env bash
    # Start the guest first: QEMU exposes its serial socket before the UART is
    # ready, so the client needs the same short delay as the standalone recipe.
    (
        cd aegis
        cargo run --release
    ) &
    kernel_pid=$!
    trap 'kill "$kernel_pid" 2>/dev/null || true' EXIT
    sleep 2
    cd client
    cargo run --release -- {{args}}

kernel:
    #!/usr/bin/env bash
    cd aegis
    status=0
    cargo run --release || status=$?
    if [[ $status -ne 0 && $status -ne 33 ]]; then
        exit "$status"
    fi

client *args:
    # QEMU creates the serial socket before the guest has initialized its UART.
    # Give the guest a short head start so the first HELLO is not truncated.
    sleep 2
    cd client && cargo run --release -- {{args}}

fmt-check:
    cargo fmt --all -- --check

check: check-kernel check-client

check-kernel:
    cd aegis && cargo check --release

check-client:
    cargo check -p client --release

lint: lint-kernel lint-client

lint-kernel:
    cd aegis && cargo clippy --release -- -D warnings

lint-client:
    cargo clippy -p client --release -- -D warnings

test:
    cargo test -p libaegis --no-default-features
    cargo test -p libaegis --features std
    cargo test -p client

validate: fmt-check check lint test
