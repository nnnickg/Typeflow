# Host Test Matrix

Run this after `make -C macos install-user` has installed and registered the
bundle. This is intentionally manual: Slack, browser password fields, Notes,
and Mail are host/editor behavior, not something the Rust unit suite can
truthfully certify.

Assumptions:

- Typeflow is selected as the current input source.
- The embedded secondary language is Ukrainian, or `language.secondary = "uk"`.
- The tested app is not listed in `apps.disable_bundle_ids` or
  `apps.disable_auto_bundle_ids`, except for the explicit policy checks.
- ABC or another standard Latin input source remains installed as fallback.

Core cases for every normal text field:

| Input | Expected visible text | Purpose |
| --- | --- | --- |
| `[fnf` | `хата` | punctuation-position secondary letters stay inside the composed token |
| `'dhj` | `євро` | ASCII quote key can be part of a secondary token |
| `’dhj` | `євро` | host smart quote mutation does not leave a leading quote behind |
| `hello —[fnf` | `hello —хата` | smart dash before token is a boundary, not token corruption |
| `ghsdbn` | `привіт` | normal Ukrainian conversion still works |
| `http` | `http` | common English technical token stays English |
| `user@example.com` | `user@example.com` | email-like text stays English |
| `arn:aws:iam::000000000000:role/ExampleRole` | unchanged | infrastructure identifier stays English |
| `CLOUDACCESSKEYIDLIKEVALUE1234` | unchanged | secret-like token stays English |

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

- Normal fields apply the core cases above.
- Password fields bypass Typeflow completely; `[fnf` must remain `[fnf`.
- Apps in `apps.disable_auto_bundle_ids` bypass automatic layout switching, but
  still let Typeflow own the active composition in the current layout.
  Standalone Option changes that active composition to the opposite layout.
  After standalone Option conversion, the next word should commit in the
  selected manual layout without automatic switching.
- Apps in `apps.disable_bundle_ids` bypass both automatic changes and
  standalone Option conversion.
- Terminal-like surfaces bypass both automatic changes and standalone Option
  conversion even when the app is not listed in config. Normal editor buffers in
  the same app should still follow the app's normal policy.
- Embedded terminal-pane detection depends on macOS Accessibility metadata. If
  Typeflow is not Accessibility-trusted, terminal apps are still blocked by
  bundle id but embedded terminal panes may look like normal editor text.
- Autocorrect or rich-text mutations may change punctuation around the token,
  but the final commit must not leave stale ASCII/smart punctuation inside the
  word.
