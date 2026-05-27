# Host Test Matrix

Run this after `make -C macos install-user` has installed and started the
agent. This is intentionally manual because host/editor behavior is outside the
Rust unit suite.

Assumptions:

- TypeClaw is running as a background agent.
- Real English and secondary keyboard input sources are installed. Configure
  `[macos].english_input_source_id` and `[macos].secondary_input_source_id` if
  auto-detection picks the wrong ones.
- The tested app is not listed in `apps.disable_bundle_ids` or
  `apps.disable_auto_bundle_ids`, except for explicit policy checks.

Core cases for every normal text field:

| Input | Expected visible text | Purpose |
| --- | --- | --- |
| `ghsdbn` | secondary word, e.g. `привіт` by configured pack | automatic token replacement |
| `[fnf` | secondary candidate when valid, otherwise unchanged | punctuation-position keys stay engine-owned, not ad hoc text parsing |
| `hello ghsdbn` | `hello ` plus replaced secondary word | space resets observed token before next replacement |
| `http` | `http` | English technical token remains untouched |
| `user@example.com` | `user@example.com` | punctuation boundaries pass through normally |
| `arn:aws:iam::000000000000:role/ExampleRole` | unchanged | infrastructure identifier stays untouched |

App/field matrix:

| App | Fields |
| --- | --- |
| TextEdit | plain text document, rich text document |
| Notes | note title, note body |
| Mail | subject, message body |
| Slack | message composer, thread reply, edit-message field |
| Safari or Chrome | normal textarea, contenteditable editor, address/search field |
| Any browser | password field |
| Terminal.app / iTerm2 | shell prompt |
| Editor with embedded terminal | editor buffer, embedded terminal pane |

Expected host behavior:

- Normal fields should feel exactly like app-native typing: no underline, no
  inline ownership UI, no overlay.
- When Rust returns `SwitchFutureLayout`, TypeClaw replaces the current token
  once and changes the real macOS keyboard source for future keys.
- Token replacement is posted as synthetic selection plus synthetic Unicode
  events. If focus changes between decision and post, TypeClaw must cancel the
  replacement. If focus changes after selection but before Unicode insertion,
  the selected text can remain selected and the token may be left unchanged.
  Treat any wrong-field replacement or wrong-field selection as a release
  blocker.
- Password fields bypass TypeClaw observation.
- Apps in `apps.disable_auto_bundle_ids` disable automatic layout switching but
  still allow standalone Option in normal non-secure fields.
- Apps in `apps.disable_bundle_ids` bypass both automatic observation behavior
  and standalone Option switching.
- Terminal-like surfaces bypass both automatic behavior and standalone Option
  switching even when the app is not listed in config.
- Embedded terminal-pane detection depends on macOS Accessibility metadata from
  the focused element and its parent containers. If TypeClaw is not
  Accessibility-trusted, terminal apps are still blocked by bundle id but
  embedded terminal panes may look like normal editor text.
