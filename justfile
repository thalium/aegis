kernel:
    cd aegis && cargo run --release

python:
    cd pyaegis && maturin develop --release

client:
    cd client && cargo run --release