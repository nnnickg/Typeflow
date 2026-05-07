# Release Verification

These checks verify the optimized Rust artifacts, not debug builds.

## Build

```sh
cargo build --release -p typeflow-cli -p typeflow-ffi
```

Expected artifacts on macOS:

```text
target/release/typeflow
target/release/libtypeflow_ffi.a
target/release/libtypeflow_ffi.dylib
target/release/libtypeflow_ffi.rlib
```

The CLI and dylib should be arm64 Mach-O files on Apple Silicon:

```sh
file target/release/typeflow target/release/libtypeflow_ffi.dylib
```

## Runtime Smoke

Use an empty config so local user config cannot shadow the embedded defaults:

```sh
touch /tmp/typeflow-empty.toml
target/release/typeflow --config /tmp/typeflow-empty.toml model
target/release/typeflow --config /tmp/typeflow-empty.toml predict ghsdbn
target/release/typeflow --config /tmp/typeflow-empty.toml pack inspect uk
target/release/typeflow --config /tmp/typeflow-empty.toml eval --generated 500
```

Expected:

- `model` reports embedded English and Ukrainian.
- `predict ghsdbn` returns `Ukrainian	привіт`.
- `pack inspect uk` reports `path: embedded`.
- generated eval passes with no failures. Ambiguous exact-English secondary
  cases may be skipped and reported.

## Release Tests

```sh
cargo test --release --workspace
```

This must include the FFI ABI smoke tests:

```text
Running tests/abi_smoke.rs
```

## FFI Symbols

```sh
nm -gU target/release/libtypeflow_ffi.dylib
```

Expected exported symbols:

```text
_typeflow_engine_current_layout
_typeflow_engine_default_config
_typeflow_engine_force_switch_token
_typeflow_engine_free
_typeflow_engine_new_embedded
_typeflow_engine_new_embedded_with_config
_typeflow_engine_new_from_data_dir
_typeflow_engine_new_from_data_dir_with_config
_typeflow_engine_new_from_pack_dir
_typeflow_engine_new_from_pack_dir_with_config
_typeflow_engine_process
_typeflow_engine_reset_layout
_typeflow_engine_reset_token
_typeflow_engine_set_host_context
```

## Packaging Caveat

Current raw release dylib install name is produced by Cargo and points into the
local `target` directory:

```sh
otool -D target/release/libtypeflow_ffi.dylib
```

Before bundling the macOS app, the packaging step must rewrite the install name
to an app-relative value, for example:

```sh
install_name_tool -id @rpath/libtypeflow_ffi.dylib target/release/libtypeflow_ffi.dylib
```

Do this in the macOS packaging/build script, not by committing mutated binary
output.
