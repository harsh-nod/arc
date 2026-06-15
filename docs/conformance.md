---
layout: default
title: Conformance
---

# Conformance and Testing

ARC includes unit, integration, conformance, fuzz-smoke, and differential tests.
The goal is to keep the textual IR, verifier, interpreter, optimizer, lowering,
and code generators aligned as the prototype evolves.

## Local Release Checks

Run these before publishing a branch:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo build --release --workspace
```

## CLI Smoke Tests

```bash
arcc verify examples/hello.air
arcc verify examples/send_email.air
arcc verify --extended examples/send_email.air
arcc audit examples/send_email.air
arcc security examples/send_email.air --confidential to
arcc codegen examples/hello.air --target wasm32
arcc codegen examples/hello.air --target x86_64
```

The confidentiality example should fail with an illegal-flow report because
`email.send` has a network effect.

## Test Corpus

The repository test corpus is organized by behavior:

- `tests/parse`: parser and printer round trips
- `tests/verify`: invalid modules rejected by the verifier
- `tests/interp`: interpreter semantics
- `tests/opt`: optimization preservation
- `tests/lower`: lowering behavior
- `tests/codegen`: x86-64 and WebAssembly codegen smoke tests
- `tests/spec`: small spec-derived examples

## Fuzz Smoke Tests

```bash
arcc fuzz-smoke --seeds 100
```

The fuzz smoke test generates deterministic valid and garbage-like inputs. It
checks that parsing, verifying, and binary decoding do not panic.

## Differential Tests

The `arc_difftest` crate compares interpreter behavior with native codegen for
the supported numeric subset. It also contains canonical conformance cases for
valid modules, invalid modules, semantic execution, and optimization behavior.

## CI

The repository includes a GitHub Actions CI workflow that runs formatting,
Clippy, tests, and rustdoc. The GitHub Pages workflow builds the `docs/`
directory with Jekyll and deploys it to Pages.

