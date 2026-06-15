---
layout: default
title: CLI Reference
---

# ARC CLI Reference

The command-line package is `arc_cli`; the executable name is `arcc`.

From a source checkout:

```bash
cargo run -p arc_cli -- <command>
```

After installing:

```bash
cargo install --path crates/arc_cli
arcc <command>
```

## Common Commands

| Command | Purpose |
|---------|---------|
| `arcc parse FILE --print` | Parse an ARC file and print canonical text |
| `arcc verify FILE` | Verify SSA, types, resources, proofs, and authority |
| `arcc verify --extended FILE` | Run base verification plus security, memory, and proof integration checks |
| `arcc run FILE` | Interpret `@main` |
| `arcc trace FILE` | Interpret `@main` and print a trace |
| `arcc run FILE --trace trace.json` | Save a JSON execution trace |
| `arcc replay trace.json` | Load and print a saved trace |
| `arcc trace-compare A.json B.json` | Compare observable trace behavior |
| `arcc opt FILE --passes LIST` | Run optimization passes |
| `arcc lower FILE --to arc-cfg` | Lower structured operations to simpler ARC CFG form |
| `arcc codegen FILE --target wasm32` | Emit WebAssembly |
| `arcc codegen FILE --target x86_64` | Emit x86-64 ELF object code |
| `arcc audit FILE` | Summarize capability and effect posture |
| `arcc security FILE` | Run information-flow, taint, and sandbox checks |
| `arcc fuzz-smoke --seeds N` | Run deterministic parser/verifier fuzz smoke tests |
| `arcc test FILE_OR_DIR...` | Batch parse/verify/run `.air` and `.arc` files |

## Parse

```bash
arcc parse examples/hello.air --print
```

`parse --print` is useful for formatter-style round trips. The printer emits
canonical ARC text and includes inferred capability effects and output types on
`arc.invoke` operations.

## Verify

```bash
arcc verify examples/send_email.air
arcc verify --extended examples/send_email.air
```

Base verification checks:

- every function has reachable blocks
- SSA values are defined before use and dominate their uses
- arithmetic, calls, branches, memory operations, and structured regions are typed
- resource values are used linearly
- proof-carrying operations only use established facts
- capability invocations reference declared capabilities
- capability invocations have an available matching `!arc.auth<capability>` token

Extended verification adds:

- security audit checks
- information-flow and taint checks
- memory model checks
- proof obligation integration

## Run And Trace

```bash
arcc run examples/hello.air
arcc trace examples/send_email.air
arcc run examples/send_email.air --trace send_email.trace.json
arcc replay send_email.trace.json
arcc explain send_email.trace.json --event email.send
```

The interpreter uses deterministic fake capability providers. Capability calls
return default values but still produce trace events and audit data.

## Optimize

```bash
arcc opt examples/hello.air --passes const_fold,dce,cse,strength_reduce
```

Available passes:

- `const_fold`
- `canonicalize`
- `dce`
- `cse`
- `inline`
- `strength_reduce`
- `simplify_cfg`

By default, the pass manager verifies the module after each pass. Use
`--no-verify-each` only while debugging a pass.

## Lower

```bash
arcc lower examples/send_email.air --to arc-cfg
```

The current lowering pipeline runs:

1. structured control flow to CFG
2. async operations to sequential calls
3. capability invokes to call stubs
4. proof erasure to runtime assertions

## Codegen

```bash
arcc codegen examples/hello.air --target wasm32
arcc codegen examples/hello.air --target x86_64
```

Targets:

- `wasm32`
- `wasm32-arc`
- `x86_64`
- `x86_64-arc-linux-elf`

Generated artifacts are written next to the input file as `.wasm` or `.o`.
Those artifacts are ignored by git.

## Security

```bash
arcc audit examples/send_email.air
arcc security examples/send_email.air --confidential to
arcc security examples/send_email.air --tainted body
arcc security examples/send_email.air --sandbox "!network.*"
```

Value annotations use SSA names without `%`. Sandbox rules can deny capability
names or effect names. For example, `!network.*` denies operations with network
effects.

