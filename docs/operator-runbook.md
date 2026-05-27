# Operator Runbook

## Logs

TypeClaw logs through macOS unified logging.

```sh
log stream --predicate 'subsystem == "io.github.nnnickg.typeclaw.agent"' --style compact
log show --last 10m --predicate 'subsystem == "io.github.nnnickg.typeclaw.agent"' --style compact
```

Performance logging uses category `Performance`. Slow calls are logged by
default. To log every measured path:

```sh
TYPECLAW_PERF_LOG_ALL=1 "$HOME/Applications/TypeClaw.app/Contents/MacOS/TypeClaw"
```

## Config

Dump the effective merged CLI/host config:

```sh
typeclaw config show
typeclaw --config /path/to/config.toml config show
```

Check the active model source and secondary language metadata:

```sh
typeclaw model
typeclaw pack list
typeclaw pack inspect uk
typeclaw pack inspect "$(typeclaw config show | sed -n 's/^secondary = "\(.*\)"/\1/p')"
```

The embedded default secondary language is Ukrainian, and
`typeclaw pack inspect uk` reports its embedded metadata. The final command
inspects the active secondary side when it is an installed local pack.

## Permissions

The app needs both Accessibility and Input Monitoring.

Programmatic checks from a local build:

```sh
log show --last 10m --predicate 'subsystem == "io.github.nnnickg.typeclaw.agent" && eventMessage CONTAINS "input monitoring"' --style compact
log show --last 10m --predicate 'subsystem == "io.github.nnnickg.typeclaw.agent" && eventMessage CONTAINS "accessibility"' --style compact
```

Runtime behavior:

- If Input Monitoring is missing at startup, TypeClaw exits after showing an
  alert.
- If Input Monitoring is revoked while TypeClaw is running, the agent disables
  event processing and posts a local notification.
- If Accessibility is missing, TypeClaw keeps bundle-id policy but cannot refine
  embedded terminal surfaces through AX metadata.

## App Lifecycle

Install for the current user:

```sh
make -C macos install-user CARGO_TARGET_DIR="$PWD/target"
```

`install-user` stops any running TypeClaw process before replacing the app
bundle, then opens the installed copy.

Release builds are ad-hoc signed. There is intentionally no hardened-runtime or
notarization path in the release artifact.
