# Panic And Unsafe Audit

Audit date: 2026-05-09.

## Commands

```sh
rg -n "unwrap\\(|expect\\(|panic!|unreachable!|todo!|unimplemented!" \
  crates/typeclaw-core/src crates/typeclaw-host-config/src crates/typeclaw-ffi/src crates/typeclaw-cli/src crates/typeclaw-data/src

rg -n "unsafe" \
  crates/typeclaw-core/src crates/typeclaw-host-config/src crates/typeclaw-ffi/src crates/typeclaw-cli/src crates/typeclaw-data/src
```

## Result

- `typeclaw-core` hot path has no `unsafe`.
- `typeclaw-cli` has no production `unwrap`, `expect`, or `panic`.
- `typeclaw-data` has no production `unwrap`, `expect`, or `panic`.
- `typeclaw-host-config` has no production `unwrap`, `expect`, `panic`, or
  `unsafe`.
- Remaining `unwrap` / `panic` sites are in `#[cfg(test)]` test code or
  test-only synthetic language helpers.
- All `unsafe` is isolated to `typeclaw-ffi/src/lib.rs`.

## Hardened During Audit

- Removed the build-tool `expect` around the built-in Ukrainian alphabet and
  made it a normal `Result` path.
- Removed the download-cache `file_name().unwrap()` logging path.
- Removed the impossible keyboard-index `expect` from reverse mapping.
- Removed production artifact-reader `expect` calls from fixed-size numeric
  decoding.
- Guarded the Swift `AXUIElement` CoreFoundation downcast with an explicit
  `CFTypeID` check at the accessibility boundary.
- Added shared engine-config validation plus FFI boundary tests for invalid
  config, null engine processing, and null default-config output.
- Moved host-config parsing/policy decisions into `typeclaw-host-config`, so
  Swift no longer carries a duplicate TOML parser or app-policy implementation
  and `typeclaw-core` stays focused on engine/data logic.
- Added `catch_unwind` guards around exported FFI functions so Rust panics do
  not unwind into Swift/AppKit.

## FFI Unsafe Boundary

The FFI layer uses `unsafe extern "C"` because it accepts raw pointers from a
host language. The implementation pattern is:

- null-tolerant functions return early or write nothing;
- constructors return null on invalid config or data-loading failure;
- C strings are decoded once through `CStr::from_ptr` after a null check;
- engine pointers are accessed through `as_ref` / `as_mut` after null checks;
- exported functions catch Rust panics at the FFI boundary, set
  `typeclaw_last_error_message`, and return the ABI's null/default value;
- `typeclaw_engine_free` and `typeclaw_host_config_free` are the only
  `Box::from_raw` sites;
- `TcObservation` stores only a state tag and layout byte. No text buffer crosses
  the hot-path FFI boundary.

Known contract:

- Passing a pointer not returned by a TypeClaw constructor is undefined
  behavior.
- Double-free is undefined behavior.
- A non-null C string pointer must point to a valid NUL-terminated string for
  the duration of the call.
- functions that return data through an out pointer require a writable pointer
  to return data; null out pointers are tolerated and leave caller memory
  untouched.

These preconditions are documented in the Rust FFI comments and summarized in
`docs/invariants.md`.

## Bounded Indexing

Reviewed direct indexing sites:

- `KeyboardMap::render` indexes fixed arrays through `PhysicalKey::index`.
  `PhysicalKey` is a closed enum with `COUNT = 34`.
- `TcObservation::write` only writes fixed-size scalar fields.
- Shared engine-config validation rejects zero lengths, `min_token_len` greater
  than `max_token_len`, non-finite/negative score floats, and `max_token_len`
  values above `MAX_CONFIG_TOKEN_LEN`.
- eval confusion-matrix indexing uses `layout_index`, which only returns `0`
  or `1`.
- CLI argument indexing is guarded by arity checks before access.
- `typeclaw-data` n-gram window indexing is guarded by `window.len() >= 2`.
- `human_bytes` unit indexing is bounded by `unit < UNITS.len() - 1`.

## Packaging Note

The raw release dylib install name is a packaging concern, not a panic/unsafe
issue. The current macOS app links the Rust static archive; standalone dylib
consumers should follow `docs/release-verification.md`.
