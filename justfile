set positional-arguments

default:
    @just --list

# Build clamor binary (release mode)
build:
    cargo build --release

# Build clamor binary (debug mode)
build-debug:
    cargo build

# Run clamor dashboard
run *FLAGS:
    cargo run -- "$@"

# Run tests
test:
    cargo test
