# Privacy

TypeClaw asks macOS for Input Monitoring and Accessibility because that is the
only practical way for a background app to observe key events, inspect the
foreground text surface, switch the real input source, and replace a wrong-layout
token.

What TypeClaw reads:

- Physical keyboard events from the macOS event tap.
- The focused app bundle id and a small set of Accessibility fields used to
  avoid secure inputs and terminal-like surfaces.
- Local config and local language-pack files.

What TypeClaw stores:

- Current in-memory token state while the app is running.
- User config under `~/.config/typeclaw/`.
- Optional installed language packs under the configured pack directory.

What TypeClaw does not do:

- It does not send keystrokes, tokens, app names, or config anywhere.
- It does not run telemetry.
- It does not write typed tokens to disk.
- It does not keep a history of replaced text.

Logs use macOS unified logging under subsystem
`io.github.nnnickg.typeclaw.agent`. Host-policy facts that may include app or
Accessibility context are logged with private privacy where possible. Performance
logs contain timings and operation names, not token text.

External language data is downloaded only when you run `typeclaw-data` to build
artifacts or packs. The released app and CLI run from embedded artifacts plus
whatever packs you install locally.
