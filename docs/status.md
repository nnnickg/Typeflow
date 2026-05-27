# Status

TypeClaw is currently a public alpha pass-through observer agent.

The default embedded secondary language is Ukrainian. Other secondary languages
come from local packs.

## Current Architecture

- Rust owns token tracking, scoring, and future layout state.
- Swift observes normal printable keys with a listen-only `CGEventTap`, sends
  `TcEvent` to Rust, and leaves insertion to macOS/the target app.
- `SwitchFutureLayout` replaces the current token once with the Rust-rendered
  target candidate, then selects a configured real macOS keyboard input source
  for future keys.
- Host policy/AX refresh is asynchronous. Key handling reads only the cached
  policy and defaults to bypass while policy is unknown or stale.
- No live inline composition is owned by TypeClaw.
- No host composition or overlay text is used for normal observed typing.
- Standalone Option converts the current tracked token when one exists, toggles
  future layout, and resets the observed token.

## Public Hot Path

- Core: `Engine::observe(InputEvent) -> ObservationOutput`
- Core manual switch: `Engine::force_switch_layout()`
- FFI: `typeclaw_engine_observe(engine, TcEvent, *out TcObservation)`
- FFI replacement snapshot:
  `typeclaw_engine_pending_replacement_delete_count` +
  `typeclaw_engine_pending_replacement_utf8_len` +
  `typeclaw_engine_take_pending_replacement_utf8`
- FFI inverse replacement snapshot for manual toggles:
  `typeclaw_engine_pending_replacement_inverse_utf8_len` +
  `typeclaw_engine_copy_pending_replacement_inverse_utf8`
- FFI manual switch:
  `typeclaw_engine_force_switch_layout(engine, *out TcObservation)`
- Swift: `TypeClawObservationAction`

`TcObservation` contains only a tag and layout. Replacement text crosses the FFI
boundary only through the explicit pending-replacement snapshot API.

## Host Policy

Rust still owns host policy. Swift only supplies facts.

- `disable_bundle_ids`: full bypass, including manual switching.
- `disable_auto_bundle_ids`: automatic switching disabled; manual Option still
  allowed in normal non-secure fields.
- secure input and terminal-like surfaces: full bypass.

There is no live-rendering policy.

## Verified

- `cargo test --workspace --locked`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `make -C macos bundle CARGO_TARGET_DIR="$PWD/target"`
- `make -C macos swift-package CARGO_TARGET_DIR="$PWD/target"`

SwiftPM may need permission to write user-level Swift/Clang cache directories.

## Important Tradeoff

The macOS host now has one mutation path: token replacement after Rust returns a
switch decision. This keeps IMK composition and overlay UI out of the system,
but it still means app/editor behavior matters. Terminals, secure input, and
disabled hosts remain bypassed.
