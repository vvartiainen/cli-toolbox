binary := "cli-toolbox"
entrypoint := "./cmd/cli-toolbox"

# Show available recipes
default:
    @just --list

# Build the CLI into ./bin
build:
    mkdir -p ./bin
    go build -o ./bin/{{binary}} {{entrypoint}}

# Run all tests
test:
    go test ./...

# Format Go source files
fmt:
    go fmt ./...
