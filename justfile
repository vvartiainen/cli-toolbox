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

# Start the agent-sandbox MicroVM (Apple silicon)
sandbox *args:
    nix run ./sandbox {{args}}

# SSH into a running agent-sandbox VM
sandbox-ssh *args:
    nix run ./sandbox#ssh -- {{args}}
