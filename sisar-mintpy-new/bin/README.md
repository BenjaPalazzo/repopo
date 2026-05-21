# bin/

Place the pre-compiled `mintpy-runner` binary here before building the Docker image.

## How to build

```bash
cd ../mintpy-runner
cargo build --release
cp target/release/mintpy-runner ../bin/mintpy-runner
```

Requires Rust 1.78+ and the `x86_64-unknown-linux-gnu` target.

For cross-compilation from macOS or Windows, use `cross`:

```bash
cargo install cross
cross build --release --target x86_64-unknown-linux-gnu
cp target/x86_64-unknown-linux-gnu/release/mintpy-runner ../bin/mintpy-runner
```

The binary is committed to the repository so that `docker build` does not
require a Rust toolchain on the build machine.
