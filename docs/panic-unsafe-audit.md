# Panic And Unsafe Audit

Audit date: 2026-05-07.

## Commands

```sh
rg -n "unwrap\\(|expect\\(|panic!|unreachable!|todo!|unimplemented!" \
  crates/typeflow-core/src crates/typeflow-ffi/src crates/typeflow-cli/src crates/typeflow-data/src

rg -n "unsafe" \
  crates/typeflow-core/src crates/typeflow-ffi/src crates/typeflow-cli/src crates/typeflow-data/src
```

## Result

- `typeflow-core` hot path has no `unsafe`.
- `typeflow-cli` has no production `unwrap`, `expect`, or `panic`.
- `typeflow-data` has no production `unwrap`, `expect`, or `panic`.
- Remaining `unwrap` / `panic` sites are in `#[cfg(test)]` test code or
  test-only synthetic language helpers.
- All `unsafe` is isolated to `typeflow-ffi/src/lib.rs`.

## Hardened During Audit

- Removed the build-tool `expect` around the built-in Ukrainian alphabet and
  made it a normal `Result` path.
- Removed the download-cache `file_name().unwrap()` logging path.
- Removed the impossible keyboard-index `expect` from reverse mapping.
- Added shared engine-config validation plus FFI boundary tests for invalid
  config, null engine processing, and null default-config output.

## FFI Unsafe Boundary

The FFI layer uses `unsafe extern "C"` because it accepts raw pointers from a
host language. The implementation pattern is:

- null-tolerant functions return early or write nothing;
- constructors return null on invalid config or data-loading failure;
- C strings are decoded once through `CStr::from_ptr` after a null check;
- engine pointers are accessed through `as_ref` / `as_mut` after null checks;
- `typeflow_engine_free` is the only `Box::from_raw` site;
- `TfAction` stores replacement text in an inline fixed buffer.

Known contract:

- Passing a pointer not returned by a Typeflow constructor is undefined
  behavior.
- Double-free is undefined behavior.
- A non-null C string pointer must point to a valid NUL-terminated string for
  the duration of the call.
- `typeflow_engine_process` requires a writable `TfAction` pointer to return
  an action.

These preconditions are documented in the Rust FFI comments and summarized in
`docs/invariants.md`.

## Bounded Indexing

Reviewed direct indexing sites:

- `KeyboardMap::render` indexes fixed arrays through `PhysicalKey::index`.
  `PhysicalKey` is a closed enum with `COUNT = 34`.
- `TfAction::write` slices `replace_text` only after checking
  `bytes.len() <= TF_REPLACE_BUF_LEN`.
- Shared engine-config validation rejects zero lengths, `min_token_len` greater
  than `max_token_len`, non-finite/negative score floats, and `max_token_len`
  values that could produce an FFI replacement larger than `TF_REPLACE_BUF_LEN`.
- eval confusion-matrix indexing uses `layout_index`, which only returns `0`
  or `1`.
- CLI argument indexing is guarded by arity checks before access.
- `typeflow-data` n-gram window indexing is guarded by `window.len() >= 2`.
- `human_bytes` unit indexing is bounded by `unit < UNITS.len() - 1`.

## Not Fixed Here

The raw release dylib install name still points into `target`. That is not a
panic/unsafe issue; it belongs to the macOS packaging step and is tracked in
`docs/release-verification.md`.
