# List available commands
_default:
    just --list

# format files
fmt:
    dprint fmt "src/*" README.md Cargo.toml

# lint cargo project
lint:
    cargo clippy

# check cargo project
check:
    cargo check

# build cargo project
build:
    cargo build

# test cargo project
test:
    cargo test

# build static musl binary
musl:
    RUSTFLAGS="" CC=musl-gcc cargo build --release --target x86_64-unknown-linux-musl
    upx -q target/x86_64-unknown-linux-musl/release/tapir

# install cargo project and create ~/bin/tp shortcut
install:
    cargo install --path .
    upx -q ~/.cargo/bin/tapir
    mkdir -p ~/bin
    echo -e '#!/bin/sh\nexec ~/.cargo/bin/tapir "$@"' > ~/bin/tp
    chmod +x ~/bin/tp

