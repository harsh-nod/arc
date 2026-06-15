# Security Policy

ARC is a prototype compiler and IR for capability-aware programs. It includes
static checks for authority tokens, capability effects, information flow, taint,
memory model violations, and proof obligations. These checks are useful for
research and testing, but ARC is not yet a hardened production sandbox.

## Reporting Vulnerabilities

Please report security issues privately through the repository security advisory
flow if available, or by contacting the maintainers listed for the repository.
Do not open a public issue for exploitable vulnerabilities or leaked credentials.

## What To Report

- Incorrect acceptance of a program that invokes a capability without matching authority
- Incorrect effect propagation or sandbox bypass
- Confidential data flow that is not reported by `arcc security`
- Parser, verifier, or binary decoder panics on untrusted input
- Build, release, or documentation workflows that expose private state

## Handling Secrets

Local tool directories such as `.codex/` are intentionally ignored. If a local
snapshot containing credentials was ever committed or published, rotate those
credentials immediately and purge the affected artifact.

