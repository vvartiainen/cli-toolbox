binary := "cli-toolbox"

# Show available recipes
default:
    @just --list

# Build the CLI into ./bin
build:
    mkdir -p ./bin
    cargo build --release
    cp target/release/{{binary}} ./bin/{{binary}}

# Run all tests
test:
    cargo test

# Format and lint Rust sources
fmt:
    cargo fmt
    cargo clippy --all-targets -- -D warnings
