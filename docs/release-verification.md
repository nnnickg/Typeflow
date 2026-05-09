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

This verifies the input-method app bundle compiles, has a valid plist, has a
generated icon resource, compiles the TIS registration/enabling helper, and is
ad-hoc signed:

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
pkill -x Typeflow
```

Expected install helper output includes:

```text
registered input source: /Users/<user>/Library/Input Methods/Typeflow.app
enabled input method: io.github.nnnickg.typeflow.inputmethod.Typeflow
updated HIToolbox enabled input sources
selected input source: io.github.nnnickg.typeflow.inputmethod.Typeflow
```

`pkill -x Typeflow` is only to force macOS to restart a running copy after the
new bundle is copied. Text Input Services starts it again when the Typeflow input
source is selected or needed.

## macOS IMK Runtime Smoke

In a normal app/text field with Typeflow selected:

1. With default embedded Ukrainian config, type `ghsdbn`; expected visible text:
   `привіт`.
2. Type `http`, then tap standalone Option; expected replacement under the
   embedded Ukrainian layout: `реез`.
3. Install any external secondary pack, set `language.secondary` to that pack
   id, restart Typeflow, and verify normal replacement plus standalone Option
   manual conversion in an app that is not disabled.
4. Press Option with another key; it should pass through as normal app input and
   must not trigger manual conversion.
5. Add an app bundle id under `[apps].disable_auto_bundle_ids`, restart
   Typeflow, and confirm automatic replacement does not fire in that app.
6. In the same auto-disabled app, tap standalone Option in a normal text field
   and confirm explicit visible-token conversion still works. Continue typing
   another word; it should commit in the selected manual layout without
   automatic replacement.
7. Add an app bundle id under `[apps].disable_bundle_ids`, restart Typeflow,
   and confirm neither automatic replacement nor standalone Option conversion
   fires. Repeat in a password field and confirm it does not fire.
8. In Terminal.app and iTerm2, type a normally-switching token such as `ghsdbn`.
   It must stay unchanged. Standalone Option must also pass through without
   conversion. Repeat in an embedded terminal pane when the host app exposes
   terminal-like accessibility metadata.

Before calling a release host-stable, run the broader app matrix in
`docs/host-test-matrix.md`. The Rust and Swift smoke tests cover engine/ABI
behavior; Slack, Notes, Mail, browsers, and password fields still require real
host/editor verification.

Known bundle IDs useful for local testing:

```toml
[apps]
disable_bundle_ids = [
    "com.googlecode.iterm2",
    "com.apple.Terminal",
]

disable_auto_bundle_ids = [
    "dev.zed.Zed",
]
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
_typeflow_engine_convert_visible_tail
_typeflow_engine_convert_visible_token
_typeflow_engine_force_switch_token
_typeflow_engine_free
_typeflow_engine_new_embedded
_typeflow_engine_new_embedded_with_config
_typeflow_engine_new_from_data_dir
_typeflow_engine_new_from_data_dir_with_config
_typeflow_engine_new_from_host_config
_typeflow_engine_new_from_pack_dir
_typeflow_engine_new_from_pack_dir_with_config
_typeflow_engine_process
_typeflow_engine_replace_visible_prefix_with_key
_typeflow_engine_replace_visible_tail_with_key
_typeflow_engine_reset_layout
_typeflow_engine_reset_token
_typeflow_engine_set_host_context
_typeflow_last_error_message
_typeflow_host_config_data_directory
_typeflow_host_config_engine_config
_typeflow_host_config_engine_source
_typeflow_host_config_auto_disabled_bundle_count
_typeflow_host_config_disabled_bundle_count
_typeflow_host_config_free
_typeflow_host_config_is_automatic_processing_disabled
_typeflow_host_config_is_bundle_disabled
_typeflow_host_config_load
_typeflow_host_config_load_defaults
_typeflow_host_config_load_with_environment
_typeflow_host_config_pack_directory
_typeflow_host_config_resolve_input_policy
_typeflow_host_config_secondary_language
_typeflow_host_config_source_path
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
