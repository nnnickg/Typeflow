# Release Verification

These checks verify the optimized Rust artifacts, not debug builds.

## Version Contract

The root `[workspace.package].version` in `Cargo.toml` is the Rust source of
truth. Workspace crates inherit it, and the CLI prints it through Cargo's
`CARGO_PKG_VERSION`.

The macOS build injects that same Cargo version into
`CFBundleShortVersionString` when it creates `TypeClaw.app`. Release tags must
be `v<version>`.

Before publishing a GitHub release:

```sh
./scripts/verify-release-version.sh v1.0.0
```

For a new release, bump `Cargo.toml`, regenerate `Cargo.lock`, commit those
changes, then create the tag from that commit. Creating a GitHub release does
not change the version embedded in any binary.

Minimal release sequence:

```sh
version=1.0.0

# edit Cargo.toml
cargo check --workspace
./scripts/verify-release-version.sh "v$version"
cargo test --workspace --locked
cargo build --release --locked -p typeclaw-cli
target/release/typeclaw -V

git add Cargo.toml Cargo.lock
git commit -m "release: v$version"
git tag -a "v$version" -m "v$version"
git push origin main "v$version"
gh release create "v$version" --title "v$version" --generate-notes
```

## Build

```sh
cargo build --release -p typeclaw-cli -p typeclaw-ffi
```

Expected artifacts on macOS:

```text
target/release/typeclaw
target/release/libtypeclaw_ffi.a
target/release/libtypeclaw_ffi.dylib
target/release/libtypeclaw_ffi.rlib
```

The CLI and dylib should be arm64 Mach-O files on Apple Silicon:

```sh
file target/release/typeclaw target/release/libtypeclaw_ffi.dylib
```

## Runtime Smoke

Use an empty config so local user config cannot shadow the embedded defaults:

```sh
touch /tmp/typeclaw-empty.toml
target/release/typeclaw --config /tmp/typeclaw-empty.toml model
target/release/typeclaw --config /tmp/typeclaw-empty.toml predict ghsdbn
target/release/typeclaw --config /tmp/typeclaw-empty.toml pack inspect uk
target/release/typeclaw --config /tmp/typeclaw-empty.toml eval --generated 500
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
single-module `swiftc` compile path. It builds `TypeClawKit`, runs the SwiftPM
staticlib smoke executable, and builds the agent executable:

```sh
make -C macos swift-package
```

Expected:

- `typeclaw-staticlib-smoke` prints
  `staticlib smoke: observed ghsdbn; host text pass-through ghsdbn`.
- `TypeClaw` builds as a SwiftPM executable linked against `libtypeclaw_ffi.a`.

## macOS Agent Bundle Build

This verifies the background agent app bundle compiles, has a valid plist, has
a generated icon resource, and is ad-hoc signed:

```sh
make -C macos bundle
```

Expected:

- `build/TypeClaw.app/Contents/Info.plist` passes `plutil -lint`.
- `build/TypeClaw.app/Contents/Resources/TypeClaw.icns` exists for Finder/Dock.
- `build/TypeClaw.app/Contents/MacOS/TypeClaw` is a Mach-O executable for the
  local build host architecture.
- `codesign --verify --strict` passes.

## macOS Universal Release Package

The release packaging path builds separate Rust and Swift artifacts for arm64
and x86_64, merges them with `lipo`, ad-hoc signs the app bundle, and writes a
zip. This is intentional: TypeClaw is distributed on a user-trust model.

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
make -C macos release-universal
```

Expected:

- `macos/build/release/TypeClaw.app/Contents/MacOS/TypeClaw` verifies both
  `arm64` and `x86_64` with `lipo -verify_arch`.
- `codesign --verify --strict` passes for the ad-hoc signature.
- `CFBundleVersion` is set to `<major version>.<git commit count>` and must not
  be `1`.
- `macos/build/release/dist/TypeClaw-macos-universal.zip` exists.

For local single-architecture smoke testing on the current machine only:

```sh
TYPECLAW_MACOS_ARCHS=arm64 make -C macos release-universal
```

The GitHub release workflow builds with `CODESIGN_IDENTITY="-"`. The packaging
scripts reject any other identity, so hardened-runtime entitlements are
intentionally absent. Users who download the zip must explicitly trust the app
on first launch.

If macOS keeps the quarantine attribute on a downloaded build, the user can
remove it after inspecting the release checksum:

```sh
xattr -dr com.apple.quarantine TypeClaw.app
```

To install and start for the current user:

```sh
make -C macos install-user
```

`install-user` stops any running TypeClaw process, copies the app to
`~/Applications/TypeClaw.app`, and opens it. On launch, installed app bundles
register the main app as a login item via `SMAppService`.
First launch requests both Accessibility and Input Monitoring. If Input
Monitoring is denied, the event tap is not created and the app exits with an
explicit permission error.

## macOS Agent Runtime Smoke

With the TypeClaw agent running and real English/secondary keyboard sources
installed:

1. With default embedded Ukrainian config, type `ghsdbn`; expected visible text:
   `привіт`. TypeClaw should replace the just-typed token once and switch the
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
   wrong source, restart TypeClaw, and verify automatic token replacement plus
   standalone Option replacement in an app that is not disabled.
5. Press Option with another key; it should pass through as normal app input and
   must not trigger manual switching.
6. Add an app bundle id under `[apps].disable_auto_bundle_ids`, restart
   TypeClaw, and confirm automatic layout switching does not fire in that app.
7. In the same auto-disabled app, tap standalone Option in a normal text field
   and confirm it can replace the current tracked token manually.
8. Add an app bundle id under `[apps].disable_bundle_ids`, restart TypeClaw,
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

CI enforces `typeclaw-core` line coverage at 40%. To reproduce it locally:

```sh
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.8.6 --locked
cargo llvm-cov -p typeclaw-core --locked --summary-only --fail-under-lines 40
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
nm -gU target/release/libtypeclaw_ffi.dylib
```

Expected exported symbols:

```text
_typeclaw_engine_current_layout
_typeclaw_engine_copy_pending_replacement_inverse_utf8
_typeclaw_engine_default_config
_typeclaw_engine_force_switch_layout
_typeclaw_engine_free
_typeclaw_engine_new_embedded
_typeclaw_engine_new_embedded_with_config
_typeclaw_engine_new_from_data_dir
_typeclaw_engine_new_from_data_dir_with_config
_typeclaw_engine_new_from_host_config
_typeclaw_engine_new_from_pack_dir
_typeclaw_engine_new_from_pack_dir_with_config
_typeclaw_engine_observe
_typeclaw_engine_pending_replacement_delete_count
_typeclaw_engine_pending_replacement_inverse_utf8_len
_typeclaw_engine_pending_replacement_utf8_len
_typeclaw_engine_reset_layout
_typeclaw_engine_reset_token
_typeclaw_engine_set_host_context
_typeclaw_engine_take_pending_replacement_utf8
_typeclaw_engine_token_len
_typeclaw_last_error_message
_typeclaw_host_config_data_directory
_typeclaw_host_config_engine_config
_typeclaw_host_config_engine_source
_typeclaw_host_config_auto_disabled_bundle_count
_typeclaw_host_config_disabled_bundle_count
_typeclaw_host_config_free
_typeclaw_host_config_is_automatic_processing_disabled
_typeclaw_host_config_is_bundle_disabled
_typeclaw_host_config_load
_typeclaw_host_config_load_defaults
_typeclaw_host_config_load_with_environment
_typeclaw_host_config_macos_english_input_source_id
_typeclaw_host_config_macos_secondary_input_source_id
_typeclaw_host_config_pack_directory
_typeclaw_host_config_resolve_input_policy
_typeclaw_host_config_secondary_language
_typeclaw_host_config_source_path
```

## Standalone Dylib Caveat

The macOS app bundle links the Rust static archive, so the app packaging script
does not ship `libtypeclaw_ffi.dylib`. Standalone dylib releases still need the
install name rewritten because Cargo points it into the local `target`
directory:

```sh
otool -D target/release/libtypeclaw_ffi.dylib
```

Before shipping a standalone dylib package, rewrite the install name to a
package-relative value, for example:

```sh
install_name_tool -id @rpath/libtypeclaw_ffi.dylib target/release/libtypeclaw_ffi.dylib
```

Do this in the standalone dylib packaging script, not by committing mutated
binary output.
