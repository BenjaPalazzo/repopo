# bin/

Place the pre-compiled `isce-runner` binary here before building the Docker image.

## How to build

```bash
cd ../isce-runner
cargo build --release
cp target/release/isce-runner ../bin/isce-runner
```

Requires Rust 1.78+ and the target `x86_64-unknown-linux-gnu`.
For cross-compilation from macOS/Windows, use cross:

```bash
cargo install cross
cross build --release --target x86_64-unknown-linux-gnu
cp target/x86_64-unknown-linux-gnu/release/isce-runner ../bin/isce-runner
```

The binary is committed to the repository so that `docker build` does not
require a Rust toolchain on the build machine.
