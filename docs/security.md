---
layout: default
title: Security
---

# Security Analysis

ARC has built-in information-flow analysis, taint tracking, and sandbox enforcement. These run at compile time over the IR, catching security violations before the program executes.

---

## Information-Flow Analysis

Every SSA value can carry a **security level**:

| Level | Can flow to |
|-------|-------------|
| `Public` | Anywhere |
| `Internal` | Internal, Confidential, Secret |
| `Confidential` | Confidential, Secret |
| `Secret` | Secret only |

Data flows "up" the lattice freely but cannot flow "down" without explicit **declassification**. If a confidential value reaches an external sink (a capability with network or filesystem effects), that's an **illegal flow**.

### Example: Detecting a Leak

```
arc.module @leaky {
  arc.capability @log.write {
    inputs(%msg: i64)
    outputs(%ok: i64)
    effects [#arc.effect<external_communication>]
    failures []
  }

  arc.func @main() -> i64 {
  ^entry:
    %secret = arc.const 42 : i64        // imagine this is confidential
    %auth = arc.require_approval %secret, %secret : !arc.auth<log.write>
    %ok = arc.invoke @log.write(%secret) // leaking it externally!
    arc.return %ok : i64
  }
}
```

```bash
$ arcc security leaky.air --confidential secret
1 security violation(s) in leaky.air:
  1. illegal flow: secret (confidential) -> public sink
```

SSA value names are passed without the leading `%`. Local annotations are kept
when labels propagate through operation results.

---

## Taint Tracking

Values can be **tainted** by their origin:

| Label | Meaning |
|-------|---------|
| `Trusted` | Internal program data |
| `UserInput` | Came from user input |
| `ThirdParty` | From external/untrusted source |
| `ModelGenerated` | Produced by an LLM |
| `NetworkInput` | From untrusted network |

Taints propagate through operations: if any operand is tainted, the result is tainted. The analysis flags when tainted values reach security-critical operations like `require_approval`.

```bash
$ arcc security program.air --tainted user_input
# Detects: tainted value 'user_input' used in require_approval
```

---

## Sandbox Policies

Restrict which capabilities a program can invoke:

```bash
# Allow only filesystem reads, deny everything else
$ arcc security program.air --sandbox "fs.read"

# Allow filesystem, deny network explicitly
$ arcc security program.air --sandbox "fs.read,fs.write,!network"

# Set maximum security level for the sandbox
$ arcc security program.air --sandbox "fs.read" --max-level internal
```

The `!` prefix explicitly denies a capability. Capabilities not in the allow-list are implicitly denied.

---

## Module Audit

The `audit` command provides a complete security posture report:

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

This tells you at a glance:
- How many external actions the program performs
- Whether it obtained approval for each
- What capabilities it uses
- Whether it handles credentials (which need extra care)
- Any security violations detected

---

## Programmatic API

The security analysis is available as a Rust API:

```rust
use arc_security::{
    SecurityContext, SecurityLevel, TaintLabel,
    SandboxPolicy, check_information_flow, audit_module,
};

// Build a security context
let mut ctx = SecurityContext::new();
ctx.set_level("secret_data", SecurityLevel::Confidential);
ctx.add_taint("user_input", TaintLabel::UserInput);

// Optionally restrict capabilities
let mut policy = SandboxPolicy::new(SecurityLevel::Internal);
policy.allow("fs.read");
policy.deny("network");

// Run analysis
let violations = check_information_flow(&module, &ctx, Some(&policy));

// Or get a full audit
let audit = audit_module(&module, &ctx, Some(&policy));
```

---

## Violation Types

| Violation | Meaning |
|-----------|---------|
| `IllegalFlow` | Data at level X flowed to a sink at level Y where X > Y |
| `TaintedUse` | Tainted value used in a security-critical operation |
| `SandboxViolation` | Capability invoked outside the allowed set |
| `CredentialLeak` | Credential-handling data flowed to logging/tracing |
| `AuthorityMismatch` | Authority token doesn't match the required capability |

---

## Attenuated Capabilities

Capabilities can be narrowed in scope for defense-in-depth:

```rust
use arc_security::AttenuatedCapability;

// Start with full fs.write capability
let mut cap = AttenuatedCapability::new("fs.write", SecurityLevel::Internal);

// Restrict to only "append" operations on internal-level data
cap.allow_operation("append");

// Validate an invocation
cap.validate("append", SecurityLevel::Public)?;   // OK
cap.validate("overwrite", SecurityLevel::Public)?; // Error: operation not allowed
cap.validate("append", SecurityLevel::Secret)?;    // Error: data level too high
```
