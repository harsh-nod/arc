# ARC: Agent Representation Compiler

ARC is a compiler intermediate representation designed for programs that interact with the real world. Unlike LLVM IR or MLIR, ARC treats external actions as **first-class operations** with declared effects, failure modes, and authority requirements.

Current status: ARC is a working prototype, not a complete LLVM/MLIR replacement. The repository has executable support for a focused subset: parsing/printing, core verification, interpretation and traces, optimization passes with verifier checks, lowering, basic x86-64/wasm codegen, capability-provider scaffolding, security analysis, memory analysis, and package/binary-format experiments. Several roadmap items are still partial or model-level implementations.

```
arc.capability @email.send {
  inputs(%to: i64, %body: i64)
  outputs(%status: i64)
  effects [#arc.effect<network>, #arc.effect<irreversible>]
  failures [#arc.fail<delivery_error>]
}

arc.func @main() -> i64 {
^entry:
  %to   = arc.const 1 : i64
  %body = arc.const 2 : i64
  %auth = arc.require_approval %to, %body : !arc.auth<email.send>
  %status = arc.invoke @email.send(%to, %body)
  arc.return %status : i64
}
```

In LLVM, `email.send` is an opaque function call. In ARC, the compiler knows it's irreversible, has network effects, can fail, and requires approval.

## Implemented Prototype Features

- **Capabilities** -- External actions declared with typed inputs/outputs, effects, and failure modes
- **Authority** -- `require_approval` operations tracked by the verifier and enforceable at runtime
- **Effect System** -- 18 effect categories (network, filesystem, financial, credential, irreversible, ...) with commutativity analysis
- **Security Analysis** -- Information-flow tracking, taint propagation, sandbox enforcement at compile time
- **Proof-Carrying Code** -- Proof obligations for bounds checks, preconditions, and authority requirements
- **Async Primitives** -- `spawn`, `await`, `checkpoint` as IR-level operations
- **Execution Traces** -- Full audit trails with effect and authority events for every execution

## Quick Start

```bash
cargo build --release

# Install the CLI locally if desired
cargo install --path crates/arc_cli

# Run a program
cargo run -p arc_cli -- run examples/hello.air           # -> 10
arcc run examples/hello.air                              # -> 10

# Verify correctness
cargo run -p arc_cli -- verify examples/send_email.air

# Include security, memory, and proof integration checks
cargo run -p arc_cli -- verify --extended examples/send_email.air

# Security audit
cargo run -p arc_cli -- audit examples/send_email.air

# Compile to WebAssembly
cargo run -p arc_cli -- codegen examples/hello.air --target wasm32

# Compile to x86-64
cargo run -p arc_cli -- codegen examples/hello.air

# Optimize, verifying after each pass by default
cargo run -p arc_cli -- opt examples/hello.air --passes const_fold,dce

# Information-flow analysis
cargo run -p arc_cli -- security program.air --confidential secret_data
```

## Documentation

| Page | Description |
|------|-------------|
| [Getting Started](docs/getting-started.md) | Install, first program, CLI walkthrough |
| [Language Reference](docs/language-reference.md) | Complete .air syntax and operation reference |
| [Examples](docs/examples.md) | Annotated programs with capabilities, effects, branching, async |
| [CLI Reference](docs/cli.md) | Command reference for `arcc` |
| [Authority and Effects](docs/authority-effects.md) | Capability effects and approval-token verification |
| [Architecture](docs/architecture.md) | Compiler pipeline, crate map, stage descriptions |
| [Security](docs/security.md) | Information-flow analysis, taint tracking, sandboxing |
| [Conformance](docs/conformance.md) | Test suites and release checks |
| [Package Format](docs/package-format.md) | Binary/package format experiments |
| [Release Guide](docs/release.md) | Maintainer release checklist |

## Pipeline

```
.air source -> Parse -> Verify -> Optimize -> Lower -> Codegen -> .o / .wasm
                                                |
                                           Interpret -> trace
```

**Lowering passes:** structured control flow -> flat CFG, async -> sequential, capability invoke -> function call, proof erasure -> runtime assertions.

**Codegen targets:** x86-64 (ELF), WebAssembly.

## Design Goals

The table below describes the intended direction. The current implementation covers usable subsets of these areas.

| | LLVM IR | MLIR | ARC |
|---|---------|------|-----|
| Capabilities | -- | -- | Declared with effects, failures, authority |
| Effect tracking | -- | Side-effect traits | 18-category lattice with commutativity |
| Authority | -- | -- | `require_approval` + verifier enforcement |
| Information flow | -- | -- | 4-level classification + taint tracking |
| Proofs | -- | -- | First-class proof objects |
| Async | -- | -- | IR-level spawn/await/checkpoint |
| Audit trails | -- | -- | Full execution traces |

## Project Structure

```
crates/
  arc_ir/        Core IR (Module, Function, Block, Operation, Type)
  arc_syntax/    Parser and printer
  arc_verify/    Verification (SSA, types, effects, authority)
  arc_interp/    Reference interpreter with traces
  arc_pass/      Optimization passes
  arc_lower/     Lowering passes
  arc_codegen/   Code generation (x86-64, WebAssembly)
  arc_security/  Information-flow analysis and sandboxing
  arc_effects/   Effect lattice
  arc_proof/     Proof obligation system
  arc_cli/       Command-line interface
  ...            (22 crates total)
```

## Tests

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

## License

Apache-2.0
