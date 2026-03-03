kernel:
    cd aegis && cargo run --release

python:
    cd pyaegis && maturin develop

client:
    cd client && cargo run --release