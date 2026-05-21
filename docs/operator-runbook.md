# Operator Runbook

## Logs

Typeflow logs through macOS unified logging.

```sh
log stream --predicate 'subsystem == "io.github.nnnickg.typeflow.agent"' --style compact
log show --last 10m --predicate 'subsystem == "io.github.nnnickg.typeflow.agent"' --style compact
```

Performance logging uses category `Performance`. Slow calls are logged by
default. To log every measured path:

```sh
TYPEFLOW_PERF_LOG_ALL=1 "$HOME/Applications/Typeflow.app/Contents/MacOS/Typeflow"
```

## Config

Dump the effective merged CLI/host config:

```sh
typeflow config show
typeflow --config /path/to/config.toml config show
```

Check the active model source and installed secondary pack metadata:

```sh
typeflow model
typeflow pack list
typeflow pack inspect uk
typeflow pack inspect "$(typeflow config show | sed -n 's/^secondary = "\(.*\)"/\1/p')"
```

## Permissions

The app needs both Accessibility and Input Monitoring.

Programmatic checks from a local build:

```sh
log show --last 10m --predicate 'subsystem == "io.github.nnnickg.typeflow.agent" && eventMessage CONTAINS "input monitoring"' --style compact
log show --last 10m --predicate 'subsystem == "io.github.nnnickg.typeflow.agent" && eventMessage CONTAINS "accessibility"' --style compact
```

Runtime behavior:

- If Input Monitoring is missing at startup, Typeflow exits after showing an
  alert.
- If Input Monitoring is revoked while Typeflow is running, the agent disables
  event processing and posts a local notification.
- If Accessibility is missing, Typeflow keeps bundle-id policy but cannot refine
  embedded terminal surfaces through AX metadata.

## App Lifecycle

Install for the current user:

```sh
make -C macos install-user CARGO_TARGET_DIR="$PWD/target"
```

Restart after replacing the app bundle:

```sh
pkill -x Typeflow
open -g "$HOME/Applications/Typeflow.app"
```

Release builds are ad-hoc signed. There is intentionally no hardened-runtime or
notarization path in the release artifact.
