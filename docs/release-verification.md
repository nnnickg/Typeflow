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
staticlib smoke: observed ghsdbn; host text pass-through ghsdbn
```

## macOS SwiftPM Build

This verifies the Swift library/executable target graph outside the Makefile's
single-module `swiftc` compile path. It builds `TypeflowKit`, runs the SwiftPM
staticlib smoke executable, and builds the agent executable:

```sh
make -C macos swift-package
```

Expected:

- `typeflow-staticlib-smoke` prints
  `staticlib smoke: observed ghsdbn; host text pass-through ghsdbn`.
- `Typeflow` builds as a SwiftPM executable linked against `libtypeflow_ffi.a`.

## macOS Agent Bundle Build

This verifies the background agent app bundle compiles, has a valid plist, has
a generated icon resource, and is ad-hoc signed:

```sh
make -C macos bundle
```

Expected:

- `build/Typeflow.app/Contents/Info.plist` passes `plutil -lint`.
- `build/Typeflow.app/Contents/Resources/Typeflow.icns` exists for Finder/Dock.
- `build/Typeflow.app/Contents/MacOS/Typeflow` is a Mach-O executable for the
  local build host architecture.
- `codesign --verify --strict` passes.

## macOS Universal Release Package

The release packaging path builds separate Rust and Swift artifacts for arm64
and x86_64, merges them with `lipo`, signs the app bundle, and writes a zip:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
make -C macos release-universal CODESIGN_IDENTITY="Developer ID Application: <name> (<team>)"
```

Expected:

- `macos/build/release/Typeflow.app/Contents/MacOS/Typeflow` verifies both
  `arm64` and `x86_64` with `lipo -verify_arch`.
- Codesigning uses hardened runtime and timestamp when `CODESIGN_IDENTITY` is
  not `-`.
- `macos/build/release/dist/Typeflow-macos-universal.zip` exists.

For local unsigned smoke testing on the current machine only:

```sh
TYPEFLOW_MACOS_ARCHS=arm64 make -C macos release-universal
```

To notarize, either configure an App Store Connect keychain profile:

```sh
NOTARY_PROFILE=typeflow-notary \
make -C macos release-universal CODESIGN_IDENTITY="Developer ID Application: <name> (<team>)"
```

or provide `APPLE_ID`, `APPLE_TEAM_ID`, and `APPLE_APP_PASSWORD`. The script
submits the zip with `xcrun notarytool`, staples the app bundle, and recreates
the zip after stapling.

To install and start for the current user:

```sh
make -C macos install-user
pkill -x Typeflow
```

`install-user` copies the app to `~/Applications/Typeflow.app` and opens it.
On launch, installed app bundles register the main app as a login item via
`SMAppService`.
First launch requests both Accessibility and Input Monitoring. If Input
Monitoring is denied, the event tap is not created and the app exits with an
explicit permission error.
`pkill -x Typeflow` is only to force a running copy to restart after reinstall.

## macOS Agent Runtime Smoke

With the Typeflow agent running and real English/secondary keyboard sources
installed:

1. With default embedded Ukrainian config, type `ghsdbn`; expected visible text:
   `привіт`. Typeflow should replace the just-typed token once and switch the
   real input source after the decision threshold for future keys.
2. Type `type`, then tap standalone Option; expected visible text becomes the
   configured secondary rendering (`ензу` with the embedded Ukrainian pack), and
   the future keyboard source switches to secondary. Tap standalone Option again
   before typing another token; expected visible text returns to `type`, and the
   future keyboard source switches back to English.
3. After a token boundary, tap standalone Option with no active token; expected
   no visible text mutation, but the future keyboard source toggles when manual
   switching is allowed.
4. Install any external secondary pack, set `language.secondary` to that pack
   id, configure `[macos].secondary_input_source_id` if auto-detection picks the
   wrong source, restart Typeflow, and verify automatic token replacement plus
   standalone Option replacement in an app that is not disabled.
5. Press Option with another key; it should pass through as normal app input and
   must not trigger manual switching.
6. Add an app bundle id under `[apps].disable_auto_bundle_ids`, restart
   Typeflow, and confirm automatic layout switching does not fire in that app.
7. In the same auto-disabled app, tap standalone Option in a normal text field
   and confirm it can replace the current tracked token manually.
8. Add an app bundle id under `[apps].disable_bundle_ids`, restart Typeflow,
   and confirm neither automatic observation behavior nor standalone Option switching
   fires. Repeat in a password field and confirm it does not fire.
9. In Terminal.app and iTerm2, type a normally-switching token such as `ghsdbn`.
   It must stay unchanged. Standalone Option must also pass through without
   switching. Repeat in an embedded terminal pane when the host app exposes
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
    "dev.zed.Zed",
]

disable_auto_bundle_ids = [
    "com.microsoft.VSCode",
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

## Coverage

The CI coverage job generates an LCOV artifact from the full Rust workspace:

```sh
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.8.6 --locked
cargo llvm-cov --workspace --locked --lcov --output-path lcov.info
```

## Fuzz Target Build

The fuzz harnesses are kept outside the main workspace and compile with their
own lockfile:

```sh
cargo build --manifest-path fuzz/Cargo.toml --bins --locked
```

For local fuzz campaigns, install `cargo-fuzz` and run a bounded target:

```sh
cargo install cargo-fuzz --locked
cargo fuzz run artifact_decoders -- -max_total_time=60
cargo fuzz run ffi_events -- -max_total_time=60
```

## FFI Symbols

```sh
nm -gU target/release/libtypeflow_ffi.dylib
```

Expected exported symbols:

```text
_typeflow_engine_current_layout
_typeflow_engine_copy_pending_replacement_inverse_utf8
_typeflow_engine_default_config
_typeflow_engine_force_switch_layout
_typeflow_engine_free
_typeflow_engine_new_embedded
_typeflow_engine_new_embedded_with_config
_typeflow_engine_new_from_data_dir
_typeflow_engine_new_from_data_dir_with_config
_typeflow_engine_new_from_host_config
_typeflow_engine_new_from_pack_dir
_typeflow_engine_new_from_pack_dir_with_config
_typeflow_engine_observe
_typeflow_engine_pending_replacement_delete_count
_typeflow_engine_pending_replacement_inverse_utf8_len
_typeflow_engine_pending_replacement_utf8_len
_typeflow_engine_reset_layout
_typeflow_engine_reset_token
_typeflow_engine_set_host_context
_typeflow_engine_take_pending_replacement_utf8
_typeflow_engine_token_len
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
_typeflow_host_config_macos_english_input_source_id
_typeflow_host_config_macos_secondary_input_source_id
_typeflow_host_config_pack_directory
_typeflow_host_config_resolve_input_policy
_typeflow_host_config_secondary_language
_typeflow_host_config_source_path
```

## Standalone Dylib Caveat

The macOS app bundle links the Rust static archive, so the app packaging script
does not ship `libtypeflow_ffi.dylib`. Standalone dylib releases still need the
install name rewritten because Cargo points it into the local `target`
directory:

```sh
otool -D target/release/libtypeflow_ffi.dylib
```

Before shipping a standalone dylib package, rewrite the install name to a
package-relative value, for example:

```sh
install_name_tool -id @rpath/libtypeflow_ffi.dylib target/release/libtypeflow_ffi.dylib
```

Do this in the standalone dylib packaging script, not by committing mutated
binary output.
