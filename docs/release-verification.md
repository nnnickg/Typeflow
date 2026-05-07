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

## macOS Staticlib Smoke

This verifies Swift can import the local C module map, link the Rust static
archive, and call the FFI:

```sh
make -C macos smoke
```

Expected:

```text
staticlib smoke: ghsdbn -> привіт
```

## macOS IMK Bundle Build

This verifies the minimal input-method app bundle compiles, has a valid plist
with a visible Ukrainian mode, has a generated icon resource, compiles the TIS
registration/enabling helper, and is ad-hoc signed:

```sh
make -C macos bundle
```

Expected:

- `build/Typeflow.app/Contents/Info.plist` passes `plutil -lint`.
- `build/Typeflow.app/Contents/Resources/Typeflow.icns` exists for Finder/Dock.
- `build/Typeflow.app/Contents/Resources/Typeflow.tiff` exists for TIS/input
  source menus.
- `build/Typeflow.app/Contents/MacOS/Typeflow` is an arm64 Mach-O executable on
  Apple Silicon.
- `build/typeflow-register-input-source` compiles.
- `codesign --verify --strict` passes.

To install and register for the current user:

```sh
make -C macos install-user
```

Expected install helper output includes:

```text
registered input source: /Users/<user>/Library/Input Methods/Typeflow.app
enabled input method: io.github.nnnickg.typeflow.inputmethod.Typeflow
enabled input source: io.github.nnnickg.typeflow.inputmethod.Typeflow.Ukrainian
updated HIToolbox enabled input sources
```

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
