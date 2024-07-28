#!/usr/bin/env bash
set -euo pipefail

# Resulting from https://medium.com/rust-programming-language/simplifying-debian-packaging-for-rust-a-step-by-step-guide-for-rust-developers-0457cdb3c81d
# Package config is located in Cargo.toml's [package.metadata.deb] section

# version=0.0.0
cargo install cargo-deb
cargo deb
## Install
# dpkg -i ./target/debian/containeryard_$version.deb
## Uninstall
# sudo dpkg -r containeryard