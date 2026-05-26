# Architecture

## High-Level Shape

```text
keyDown
-> Swift host reads cached host context
-> Swift translates the event to TfEvent
-> Rust observes the event and updates token/layout state
-> macOS/app inserts the key normally
-> on a switch decision, Swift replaces the just-typed token once
-> Swift schedules a real input-source switch for future keys
```

Typeflow is a pass-through state machine. It does not compose inline text, draw
an overlay, or call host insertion/replacement APIs per key. Host mutation is a
single explicit token-replacement side effect when Rust decides the active token
belongs to the other layout.

The embedded default language pair is English plus Ukrainian. External packs
replace only the secondary side.

## Crates

### `typeflow-core`

Pure Rust. Hot path has no I/O. It owns:

- `PhysicalKey`, `LetterEvent`, `InputEvent`
- token buffer and rendered layout candidates
- scoring and layout decisions
- `Engine::observe(InputEvent) -> ObservationOutput`
- `Engine::force_switch_layout() -> ObservationOutput`

The engine returns state notifications, not text:

- `ObservationAction::None`
- `ObservationAction::SwitchFutureLayout(Layout)`
- `ObservationAction::ResetToken`

`docs/invariants.md` is the contract for host behavior.

### `typeflow-host-config`

Rust config and host-policy layer shared by CLI and FFI. It owns TOML parsing,
environment overrides, default config paths, app disable policy, and host-surface
classification from bundle id, secure-input state, and Accessibility facts.

The app policy surface is intentionally small:

- `disable_bundle_ids`: full bypass, including manual Option.
- `disable_auto_bundle_ids`: automatic switching disabled, manual Option still
  allowed in normal non-secure fields.

There is no live-rendering policy because there is no live composition path.

### `typeflow-ffi`

C ABI for the macOS bundle. The hot path is:

- `typeflow_engine_observe(engine, TfEvent, *out TfObservation)`
- `typeflow_engine_force_switch_layout(engine, *out TfObservation)`
- `typeflow_engine_reset_token`
- `typeflow_engine_reset_layout`
- `typeflow_engine_set_host_context`
- `typeflow_engine_current_layout`
- `typeflow_engine_token_len`
- `typeflow_engine_pending_replacement_delete_count`
- `typeflow_engine_pending_replacement_utf8_len`
- `typeflow_engine_pending_replacement_inverse_utf8_len`
- `typeflow_engine_copy_pending_replacement_inverse_utf8`
- `typeflow_engine_take_pending_replacement_utf8`

`TfObservation` is lifetime-free and text-free:

```c
typedef struct {
    uint8_t tag;
    uint8_t layout;
} TfObservation;
```

### `typeflow-cli`

CLI tools use the same observer engine. `predict`, `stream`, and `eval` inspect
the engine's final candidates/layout; they do not simulate committed host text as
engine output. `repl` shows pass-through host text plus the observed state trace.

### `macos/`

The macOS target is an LSUIElement background agent plus Swift wrappers:

- `TypeflowKit/Engine.swift` wraps `TfEngine*` and decodes `TfObservation`.
- `TypeflowKit/HostConfig.swift` wraps opaque Rust config/policy.
- `TypeflowKit/KeyCodeMap.swift` maps ANSI virtual keycodes to Rust physical
  key indices.
- `TypeflowAgent/main.swift` installs a listen-only `CGEventTap`, observes
  keys, replaces switched tokens by selecting the previous tracked token and
  posting Unicode over that selection, and selects real macOS keyboard input
  sources for future keys. Installed app
  bundles register the main app with `SMAppService` so Typeflow launches at
  login. Startup explicitly requests Accessibility and Input Monitoring before
  creating the event tap.
- `TypeflowSmoke/main.swift` verifies the static archive and pass-through
  observer behavior.

The old compositor files are gone. There is no inline rendering path and no
per-key direct-commit path.

Startup is fail-fast. If config loading, engine construction, event-tap
creation, or run-loop source creation fails, the macOS agent reports the error
and exits non-zero rather than running as a disabled accessory process.

## Data Flow

1. The listen-only event tap receives a raw `CGEvent`.
2. Swift cancels pending standalone Option switch if a real key arrives.
3. Swift uses cached host facts/policy only. AX refresh runs asynchronously off
   the key path and publishes a fresh policy when ready.
4. Secure, terminal, and disabled surfaces reset/bypass the engine.
5. Printable letters are sent as physical-key `TfEvent`s.
6. Boundaries such as space, unambiguous punctuation, return, tab, escape,
   focus loss, and app switch reset the observed token.
7. Rust updates token candidates, scoring cache, current layout, captures a
   pending replacement snapshot for switch actions, and returns an
   `ObservationAction`.
8. On `SwitchFutureLayout`, Swift consumes the pending replacement snapshot,
   schedules one token replacement, and schedules `TISSelectInputSource` for the
   configured real keyboard source.

Standalone Option is the exception: Swift treats the modifier-only press/release
as a Typeflow command, calls `force_switch_layout()`, consumes the replacement
snapshot captured by that call when one exists, and switches the real input
source.

## Non-Goals

- Host-owned inline composition.
- Overlay impersonation of editor text.
- Per-key insertion or replacement.
- Cloud inference.
- Layouts needing keys outside the current 34-position physical-key model.
