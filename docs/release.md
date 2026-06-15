---
layout: default
title: Release Guide
---

# Release Guide

This checklist is for maintainers preparing a public ARC release.

## 1. Clean The Worktree

```bash
git status --short --untracked-files=all
```

Do not release local tool state, generated objects, generated wasm files,
private logs, shell snapshots, or credentials. The repository ignores `.codex/`,
`examples/*.o`, and `examples/*.wasm`.

## 2. Run Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo build --release --workspace
```

## 3. Run CLI Smoke Tests

```bash
cargo run -p arc_cli -- verify examples/hello.air
cargo run -p arc_cli -- verify examples/send_email.air
cargo run -p arc_cli -- verify --extended examples/send_email.air
cargo run -p arc_cli -- audit examples/send_email.air
cargo run -p arc_cli -- security examples/send_email.air --confidential to
cargo run -p arc_cli -- security examples/send_email.air --sandbox "!network.*"
cargo run -p arc_cli -- codegen examples/hello.air --target wasm32
cargo run -p arc_cli -- codegen examples/hello.air --target x86_64
```

The two `security` commands are expected to fail with violations.

## 4. Review Public Claims

Make sure README and docs describe the current executable subset:

- parsing and printing
- verification
- interpretation and traces
- optimization
- lowering
- x86-64 and wasm codegen
- capability effects
- authority tokens
- security analysis
- memory/proof/package experiments

Do not describe roadmap items as complete.

## 5. Publish GitHub Pages

The Pages workflow builds the `docs/` directory with Jekyll. In repository
settings, enable GitHub Pages with GitHub Actions as the source. After merge,
the `GitHub Pages` workflow deploys the site.

## 6. Crate Publishing Notes

The workspace crates have package metadata and versioned internal dependencies.
If publishing to crates.io, publish dependency crates first, then crates that
depend on them. Source releases do not require crates.io publishing.

## 7. Tag

```bash
git tag -a v0.1.0 -m "ARC 0.1.0"
git push origin v0.1.0
```

