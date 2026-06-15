---
layout: default
title: Examples
---

# ARC Examples

## Hello World: Arithmetic

The simplest ARC program. Two constants, an addition, a return.

```
arc.module @hello {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 3 : i64       // define constant 3
    %b = arc.const 7 : i64       // define constant 7
    %c = arc.add %a, %b : i64    // add them -> 10
    arc.return %c : i64           // return 10
  }
}
```

```bash
$ arcc run hello.air
10
```

Key ideas:
- Every value (`%a`, `%b`, `%c`) is defined exactly once (SSA form)
- Every operation has a type annotation after `:`
- `^entry` is the block label; every function needs at least one block
- `arc.return` is a **terminator** -- it must be the last operation in a block

---

## Function Calls

Functions call other functions with `arc.call`. Arguments are passed by value.

```
arc.module @call_demo {
  // A helper function that doubles its input
  arc.func @double(%x: i64) -> i64 {
  ^entry:
    %two = arc.const 2 : i64
    %r = arc.mul %x, %two : i64
    arc.return %r : i64
  }

  // Main calls double(5) and returns the result
  arc.func @main() -> i64 {
  ^entry:
    %five = arc.const 5 : i64
    %result = arc.call @double(%five)   // result = 10
    arc.return %result : i64
  }
}
```

```bash
$ arcc run call_demo.air
10
```

---

## Branching and Comparisons

Control flow uses blocks and branch operations. `arc.cond_br` dispatches on a boolean.

```
arc.module @safe_index {
  // Returns the index if in-bounds, -1 otherwise
  arc.func @bounds_check(%idx: i64, %len: i64) -> i64 {
  ^entry:
    // Compare: is idx < len?
    %in_bounds = arc.icmp slt %idx, %len : i1

    // Branch based on the comparison
    arc.cond_br %in_bounds, ^ok, ^err

  ^ok:
    arc.return %idx : i64

  ^err:
    %neg_one = arc.const -1 : i64
    arc.return %neg_one : i64
  }

  arc.func @main() -> i64 {
  ^entry:
    %idx = arc.const 3 : i64
    %len = arc.const 10 : i64
    %result = arc.call @bounds_check(%idx, %len)
    arc.return %result : i64       // returns 3 (in bounds)
  }
}
```

```bash
$ arcc run safe_index.air
3
```

The comparisons available: `eq`, `ne`, `slt` (signed <), `sle` (signed <=), `sgt` (signed >), `sge` (signed >=).

---

## Capabilities: Declaring External Actions

This is where ARC differs from every other IR. External actions are **declared** as capabilities with explicit effects and failure modes.

```
arc.module @send_email_demo {
  // Declare what "sending email" means at the IR level:
  // - It takes a recipient and body
  // - It returns a status code
  // - It has network effects and is irreversible
  // - It can fail with a delivery error
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

    // Request approval before invoking. The compiler tracks
    // that this approval was obtained. The runtime can enforce
    // interactive approval policies (ask a human, check a policy, etc.)
    %auth = arc.require_approval %to, %body : !arc.auth<email.send>

    // Invoke the capability. In LLVM this would be an opaque
    // function call. In ARC, the compiler knows exactly what
    // effects this has and can verify the program handles failures.
    %status = arc.invoke @email.send(%to, %body)

    arc.return %status : i64
  }
}
```

```bash
$ arcc audit send_email.air
Security audit for send_email.air:
  capability invocations: 1
  approval requests:      1
  capabilities used:      {"email.send"}
  external effects:       true
  handles credentials:    false
  violations:             none
```

---

## Multi-Capability Programs

Real programs use multiple capabilities. ARC tracks each one independently.

```
arc.module @file_copy_demo {
  // Reading files: filesystem effect, can fail with IO error
  arc.capability @fs.read {
    inputs(%path: i64)
    outputs(%data: i64)
    effects [#arc.effect<filesystem.read>]
    failures [#arc.fail<io_error>]
  }

  // Writing files: filesystem effect, can also fail
  arc.capability @fs.write {
    inputs(%path: i64, %data: i64)
    outputs(%bytes_written: i64)
    effects [#arc.effect<filesystem.write>]
    failures [#arc.fail<io_error>]
  }

  arc.func @main() -> i64 {
  ^entry:
    %src = arc.const 1 : i64
    %dst = arc.const 2 : i64

    // Each capability invocation gets its own approval
    %auth_read  = arc.require_approval %src, %src : !arc.auth<fs.read>
    %data       = arc.invoke @fs.read(%src)

    %auth_write = arc.require_approval %dst, %data : !arc.auth<fs.write>
    %written    = arc.invoke @fs.write(%dst, %data)

    arc.return %written : i64
  }
}
```

```bash
$ arcc audit file_copy.air
Security audit for file_copy.air:
  capability invocations: 2
  approval requests:      2
  capabilities used:      {"fs.read", "fs.write"}
  external effects:       true
  handles credentials:    false
  violations:             none
```

---

## Conditional Approval

Combine control flow with capabilities for conditional authorization:

```
arc.module @approval_flow {
  arc.capability @file.write {
    inputs(%path: i64, %data: i64)
    outputs(%written: i64)
    effects [#arc.effect<filesystem.write>]
    failures [#arc.fail<io_error>]
  }

  // Only write if the path is valid (non-zero)
  arc.func @check_and_write(%path: i64, %data: i64) -> i64 {
  ^entry:
    %zero  = arc.const 0 : i64
    %valid = arc.icmp ne %path, %zero : i1
    arc.cond_br %valid, ^approved, ^denied

  ^approved:
    %auth   = arc.require_approval %path, %data : !arc.auth<file.write>
    %result = arc.invoke @file.write(%path, %data)
    arc.return %result : i64

  ^denied:
    %neg = arc.const -1 : i64
    arc.return %neg : i64
  }

  arc.func @main() -> i64 {
  ^entry:
    %path = arc.const 42 : i64
    %data = arc.const 100 : i64
    %r = arc.call @check_and_write(%path, %data)
    arc.return %r : i64
  }
}
```

The capability is only invoked on the `^approved` path. The verifier and security analysis understand this -- the `^denied` path has no effects.

---

## Async Tasks

ARC has IR-level async primitives for task parallelism:

```
arc.module @async_demo {
  arc.func @worker() -> i64 {
  ^entry:
    %r = arc.const 42 : i64
    arc.return %r : i64
  }

  arc.func @main() -> i64 {
  ^entry:
    // Spawn runs @worker as a task, returns a handle
    %task = arc.spawn @worker()

    // Await blocks until the task completes, returns its result
    %result = arc.await %task

    arc.return %result : i64
  }
}
```

During lowering, `spawn` becomes a direct `call` (sequential execution) and `await` becomes an identity copy. A concurrent runtime would execute these differently, but the **semantics** (same return value) are preserved.

The same program is available as `examples/async_pipeline.air`:

```bash
$ arcc run examples/async_pipeline.air
42
```

---

## Structured Control Flow

For higher-level programs, ARC supports `if`/`else` and `loop` as region-based operations:

```
arc.module @structured_if {
  arc.func @abs(%x: i64) -> i64 {
  ^entry:
    %zero = arc.const 0 : i64
    %is_neg = arc.icmp slt %x, %zero : i1
    %result = arc.if %is_neg {
    ^then:
      %neg = arc.sub %zero, %x : i64
      arc.yield %neg
    } else {
    ^else:
      arc.yield %x
    }
    arc.return %result : i64
  }
}
```

These get lowered to flat CFG (`cond_br` + blocks) before codegen, but they're useful for frontends that emit structured programs.

---

## Security Analysis

Use the `security` command to find information-flow violations:

```bash
# Check if any confidential values leak externally
$ arcc security program.air --confidential secret_data

# Check for tainted input reaching approval operations
$ arcc security program.air --tainted user_input

# Restrict what capabilities a program can use
$ arcc security program.air --sandbox "fs.read,!network"
```

ARC detects four violation types:
- **Illegal flow**: confidential data reaching external sinks
- **Tainted use**: untrusted input used in `require_approval`
- **Sandbox violation**: capability invoked outside the allowed set
- **Credential leak**: sensitive data flowing to logging

The repository includes `examples/confidential_leak.air`, which verifies but
fails information-flow analysis when `%secret` is marked confidential:

```bash
$ arcc security examples/confidential_leak.air --confidential secret
1 security violation(s) in examples/confidential_leak.air:
  1. illegal flow: secret (confidential) -> public sink
```

---

## Compilation Pipeline

Every program goes through the same pipeline. Here's what happens to `file_copy.air`:

```bash
# 1. Parse and verify
$ arcc verify file_copy.air
verification succeeded

# 2. Lower (SCF -> async -> invoke -> proof erasure)
$ arcc lower file_copy.air -o lowered.air
# arc.invoke @fs.read -> arc.call @__cap_fs.read
# arc.require_approval -> arc.const 1

# 3. Compile
$ arcc codegen lowered.air --target wasm32
compiled lowered.air -> lowered.wasm (142 bytes)
```

Or do it all at once:

```bash
$ arcc codegen file_copy.air --target wasm32
# Lowering runs automatically before codegen
```

---

## Matrix Multiply

A more substantial example showing multi-function programs:

```
arc.module @matmul {
  // Dot product of 2-element vectors
  arc.func @dot_product(%a1: i64, %a2: i64, %b1: i64, %b2: i64) -> i64 {
  ^entry:
    %p1  = arc.mul %a1, %b1 : i64    // a1 * b1
    %p2  = arc.mul %a2, %b2 : i64    // a2 * b2
    %sum = arc.add %p1, %p2 : i64    // p1 + p2
    arc.return %sum : i64
  }

  arc.func @main() -> i64 {
  ^entry:
    // Matrix A = [[1, 2]], B = [[3], [4]]
    %a11 = arc.const 1 : i64
    %a12 = arc.const 2 : i64
    %b11 = arc.const 3 : i64
    %b21 = arc.const 4 : i64

    // C[1,1] = dot(A[1,:], B[:,1]) = 1*3 + 2*4 = 11
    %c11 = arc.call @dot_product(%a11, %a12, %b11, %b21)
    arc.return %c11 : i64
  }
}
```

```bash
$ arcc run matmul.air
11
```
