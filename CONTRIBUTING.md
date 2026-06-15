# Contributing to ARC

ARC is a research compiler project. Contributions should keep the executable
prototype, specification, and documentation aligned.

## Development Setup

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The CLI package is `arc_cli`; the installed binary is `arcc`.

```bash
cargo run -p arc_cli -- verify examples/hello.air
cargo run -p arc_cli -- audit examples/send_email.air
```

## Contribution Standards

- Add or update tests for every behavior change.
- Keep docs examples executable against the current parser and verifier.
- Keep public claims precise. If a feature is model-level or partial, say so.
- Do not commit generated files such as `examples/*.o`, `examples/*.wasm`, or local tool state.
- Do not commit credentials, local shell snapshots, or private `.codex/` state.

## Pull Request Checklist

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
- Docs updated when syntax, semantics, CLI behavior, or examples change

