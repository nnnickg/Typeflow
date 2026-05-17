# Contributing

PRs are welcome. Maintainer approval is required before anything merges to
`main`; passing CI is necessary but not sufficient.

Keep changes small and explain the behavioral impact. Do not include generated
build output, local cache directories, machine-specific config, credentials, or
ad-hoc signed app bundles.

Run the same checks CI runs before opening a PR:

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Report security issues through `SECURITY.md`, not a public issue.
