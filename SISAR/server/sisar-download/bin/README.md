# bin/sisar-download — Pre-compiled binary

This directory must contain the `sisar-download` ELF binary compiled for
`linux/amd64` before running `docker build`.

## Build instructions

### Native linux/amd64 (e.g. a Linux build machine or CI runner)

```bash
cd sisar-download/          # the Rust crate root (contains Cargo.toml)
cargo build --release
cp target/release/sisar-download bin/sisar-download
```

### Cross-compiling from macOS or Windows using `cross`

```bash
cargo install cross
cross build --release --target x86_64-unknown-linux-gnu
cp target/x86_64-unknown-linux-gnu/release/sisar-download bin/sisar-download
```

### Using Docker itself as a build environment (no local Rust toolchain needed)

```bash
docker run --rm \
  -v "$(pwd)":/workspace \
  -w /workspace \
  rust:1.78-slim \
  cargo build --release --manifest-path Cargo.toml

cp target/release/sisar-download bin/sisar-download
```

**Requires Rust 1.78 or later.**

Once compiled, run `docker build -t sisar/download:latest .` from the
`sisar-download/` directory.
