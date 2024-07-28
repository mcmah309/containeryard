
# Resulting from https://medium.com/rust-programming-language/simplifying-debian-packaging-for-rust-a-step-by-step-guide-for-rust-developers-0457cdb3c81d
# Package config in Cargo.toml - package.metadata.deb
cargo install cargo-deb
cargo deb
## Install
# dpkg -i ./target/debian/containeryard_<VERSION_HERE>.deb
## Uninstall
# sudo dpkg -r containeryard