---
layout: default
title: Getting Started
---

# Getting Started with ARC

## Installation

ARC is written in Rust. You need a Rust toolchain compatible with the workspace
`rust-version` in `Cargo.toml` (currently 1.76 or newer).

```bash
git clone https://github.com/harsh-nod/arc.git
cd arc
cargo build --release
cargo install --path crates/arc_cli
```

The CLI binary is `arcc`. All examples below use `arcc` directly.

## Your First Program

Create `hello.air`:

```
arc.module @hello {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 3 : i64
    %b = arc.const 7 : i64
    %c = arc.add %a, %b : i64
    arc.return %c : i64
  }
}
```

Every ARC program is a **module** (`@hello`) containing **functions** (`@main`). Functions have **blocks** (`^entry`), blocks have **operations**. Values start with `%`, symbols with `@`.

### Parse and verify

```bash
$ arcc verify hello.air
verification succeeded for hello.air
```

The verifier checks SSA correctness (every value defined exactly once, used after definition), type consistency, and that every block has a terminator.

### Run it

```bash
$ arcc run hello.air
10
```

The interpreter executes `@main` and prints the return value.

### Compile to WebAssembly

```bash
$ arcc codegen hello.air --target wasm32
compiled hello.air -> hello.wasm (73 bytes)
```

### Compile to x86-64

```bash
$ arcc codegen hello.air
compiled hello.air -> hello.o (516 bytes, 1 symbols)
```

---

## Adding Capabilities

Here's what makes ARC different from a normal IR. Let's write a program that sends an email:

```
arc.module @emailer {
  // Declare the capability: what it takes, what it returns,
  // what effects it has, and how it can fail.
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

    // Request authority to invoke the capability.
    // This is a first-class operation the verifier tracks.
    %auth = arc.require_approval %to, %body : !arc.auth<email.send>

    // Invoke the capability. The compiler knows this has
    // network + irreversible effects.
    %status = arc.invoke @email.send(%to, %body)

    arc.return %status : i64
  }
}
```

Save as `emailer.air` and run it:

```bash
$ arcc run emailer.air
0
```

The interpreter simulates capability invocations (returning 0 by default). But the important thing is what the **compiler knows**:

```bash
$ arcc audit emailer.air
Security audit for emailer.air:
  capability invocations: 1
  approval requests:      1
  capabilities used:      {"email.send"}
  external effects:       true
  handles credentials:    false
  violations:             none
```

The audit shows exactly what your program does externally. No hidden side effects.

---

## Branching and Control Flow

ARC supports standard SSA-style control flow with blocks and branches:

```
arc.module @classifier {
  arc.func @classify(%x: i64) -> i64 {
  ^entry:
    %zero = arc.const 0 : i64
    %is_positive = arc.icmp sgt %x, %zero : i1
    arc.cond_br %is_positive, ^pos, ^neg

  ^pos:
    %one = arc.const 1 : i64
    arc.return %one : i64

  ^neg:
    %neg_one = arc.const -1 : i64
    arc.return %neg_one : i64
  }

  arc.func @main() -> i64 {
  ^entry:
    %x = arc.const 42 : i64
    %r = arc.call @classify(%x)
    arc.return %r : i64
  }
}
```

Structured control flow (`arc.if` / `arc.loop`) is also supported and gets lowered to flat CFG before codegen:

```
arc.module @structured {
  arc.func @main() -> i64 {
  ^entry:
    %cond = arc.const 1 : i1
    %r = arc.if %cond {
    ^then:
      %a = arc.const 10 : i64
      arc.yield %a
    } else {
    ^else:
      %b = arc.const 20 : i64
      arc.yield %b
    }
    arc.return %r : i64
  }
}
```

---

## Running the Optimizer

ARC includes several optimization passes:

```bash
# Run constant folding
$ arcc opt hello.air --passes const_fold

# Chain multiple passes
$ arcc opt hello.air --passes const_fold,dce,cse,strength_reduce
```

Available passes: `const_fold`, `dce` (dead code elimination), `cse` (common subexpression elimination), `strength_reduce`, `canonicalize`.

---

## Lowering

The lowering pipeline transforms high-level ARC operations into simpler forms suitable for codegen:

```bash
$ arcc lower emailer.air
```

This runs four passes in sequence:

1. **Structured CF -> CFG** — `arc.if`/`arc.loop` become `cond_br`/`branch`
2. **Async -> Sequential** — `arc.spawn`/`arc.await` become `arc.call`
3. **Invoke -> Call** — `arc.invoke @cap` becomes `arc.call @__cap_cap`
4. **Proof Erasure** — `arc.assume`/`arc.prove` become runtime assertions

---

## Execution Traces

ARC can produce full execution traces for debugging and audit:

```bash
# Print trace to stdout
$ arcc trace emailer.air

# Save trace to file
$ arcc run emailer.air --trace trace.json

# Replay a saved trace
$ arcc replay trace.json

# Compare two traces
$ arcc trace-compare trace1.json trace2.json

# Filter trace events
$ arcc explain trace.json --event email.send
```

---

## Security Analysis

Run information-flow analysis to detect security violations:

```bash
# Basic audit
$ arcc audit program.air

# Mark values as confidential and check for leaks
$ arcc security program.air --confidential secret_data

# Mark values as tainted and check for unsafe use
$ arcc security program.air --tainted user_input

# Restrict capabilities via sandbox
$ arcc security program.air --sandbox "fs.read,!network"
```

The security analysis detects:
- **Illegal flows**: confidential data flowing to external sinks without declassification
- **Tainted use**: user input reaching approval operations
- **Sandbox violations**: capabilities invoked outside the allowed set
- **Credential leaks**: sensitive data flowing to logging/tracing sinks

---

## CLI Reference

| Command | Description |
|---------|-------------|
| `parse <file> [--print]` | Parse an .air file, optionally print |
| `verify <file>` | Verify SSA, types, effects, authority |
| `run <file> [--trace FILE]` | Interpret and optionally save trace |
| `trace <file>` | Run and print full execution trace |
| `opt <file> --passes LIST [-o FILE]` | Optimize with named passes |
| `lower <file> [--to TARGET] [-o FILE]` | Lower to target dialect |
| `codegen <file> [--target TARGET]` | Compile to object code |
| `security <file> [--confidential] [--tainted] [--sandbox]` | Information-flow analysis |
| `audit <file>` | Full security posture report |
| `replay <file>` | Replay a saved trace |
| `trace-compare <a> <b>` | Compare two traces |
| `explain <file> --event FILTER` | Filter trace events |
| `fuzz-smoke [--seeds N]` | Quick fuzz test |
| `test <files...> [--expect N] [--parse-only]` | Batch test .air files |

Targets for codegen: `x86_64` (default), `wasm32`.
