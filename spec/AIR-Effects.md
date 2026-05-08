# AIR Effect System

## Effect Lattice

```
pure
 ├── memory.read
 │    └── memory.write
 ├── allocate
 ├── deallocate
 ├── filesystem.read
 ├── filesystem.write
 ├── network
 ├── database.read
 ├── database.write
 ├── ui
 ├── llm
 ├── human.approval
 ├── external_communication
 ├── external_mutation
 ├── financial
 ├── credential
 ├── physical
 └── irreversible
```

Effects compose using join/meet. `pure` is the identity.

## Effect Qualifiers

- `deterministic`
- `nondeterministic`
- `idempotent`
- `reversible`
- `transactional`
- `blocking`
- `async`
- `may_fail`
- `may_timeout`
- `speculatable`
- `commutative`

Qualifiers refine scheduling legality and optimization safety.

## Declaration Rules

- Each operation enumerates its minimal effect set and qualifiers.
- Verifier checks declared effects against dialect semantics.
- Transformations may only weaken effect sets with explicit proof.

## Scheduling Constraints

- Pure operations may reorder freely.
- Operations with commuting effects may reorder when proof is supplied.
- Irreversible effects anchor evaluation order.
- `human.approval` introduces suspension points that require continuation handling.

## Runtime Enforcement

- Interpreter and runtime record effects in the trace.
- Runtime may sandbox capabilities based on effect set and policy.
- Effect elevation (e.g., from `filesystem.read` to `external_mutation`) is rejected without explicit rewrite and proof.
