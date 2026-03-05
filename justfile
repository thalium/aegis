kernel:
    cd aegis && cargo run --release

python:
    cd pyaegis && maturin develop

client:
    cd client && cargo run --release

test:
    #!/usr/bin/env bash
    set -euo pipefail
    just kernel >/tmp/aegis-kernel.log 2>&1 &
    kernel_pid=$!
    trap 'kill "$kernel_pid" 2>/dev/null || true; wait "$kernel_pid" 2>/dev/null || true' EXIT
    AEGIS_E2E=1 AEGIS_USE_EXISTING_SERVER=1 uv run pytest -q