build:
    cargo build --workspace
    cd extension && python build.py all

release:
    cargo build --workspace --release
    cd extension && python build.py all

run:
    cargo run -p wyncast-tui

test:
    cargo test --workspace

check:
    cargo clippy --workspace -- -D warnings
    cargo fmt --check --all
