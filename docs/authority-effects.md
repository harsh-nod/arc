---
layout: default
title: Authority and Effects
---

# Authority and Effects

ARC treats external actions as explicit capabilities. A capability declaration
describes the name, input types, output types, effects, and failure modes of an
external action.

```air
arc.capability @email.send {
  inputs(%to: i64, %body: i64)
  outputs(%status: i64)
  effects [#arc.effect<network>, #arc.effect<irreversible>]
  failures [#arc.fail<delivery_error>]
}
```

## Authority Tokens

Invoking a capability requires an available authority token whose type matches
the capability name:

```air
%auth = arc.require_approval %to, %body : !arc.auth<email.send>
%status = arc.invoke @email.send(%to, %body)
```

The token does not need to be passed as an operand to `arc.invoke`. It must be
defined in a position that dominates the invoke. This keeps the textual IR
compact while still allowing the verifier to reject missing or wrong authority.

Rejected:

```air
%status = arc.invoke @email.send(%to, %body)
```

Rejected:

```air
%auth = arc.require_approval %to, %body : !arc.auth<fs.read>
%status = arc.invoke @email.send(%to, %body)
```

## Effect Propagation

When the parser sees an `arc.invoke`, it resolves the target capability and
attaches the declared effects and output types to the operation. Security
analysis also resolves capability effects directly from the module so manually
constructed modules are checked consistently.

For `email.send`, audit reports:

```text
capability invocations: 1
approval requests:      1
external effects:       true
```

## Effect Categories

The prototype effect lattice includes:

- `pure`
- `memory.read`
- `memory.write`
- `allocate`
- `deallocate`
- `filesystem.read`
- `filesystem.write`
- `network`
- `database.read`
- `database.write`
- `ui`
- `llm`
- `human.approval`
- `external_communication`
- `external_mutation`
- `financial`
- `credential`
- `physical`
- `irreversible`

External effects are treated as observable boundary crossings. Security checks
use them as sinks for confidential data unless the value is explicitly
declassified.

## Optimizer Contract

Pure arithmetic can be folded, eliminated, or common-subexpression-eliminated.
Effectful operations are preserved. Capability invokes, approvals, calls,
assertions, memory writes, and allocation operations are conservatively kept.

## Sandbox Rules

Sandbox rules can deny or allow capability names:

```bash
arcc security program.air --sandbox "email.send,!file.write"
```

Rules can also deny effect classes:

```bash
arcc security program.air --sandbox "!network.*"
```

The deny list has priority over the allow list.

