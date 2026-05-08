# AIR Core Specification

## Overview

This document defines the core structural and semantic rules of the AIR intermediate representation.
AIR applies an SSA discipline extended with explicit resource values, dependent indices, and proof objects.
All transformations and runtime components must observe the contracts declared here.

## Modules and Symbols

- An AIR module is introduced with `air.module @name { ... }`.
- Modules own symbol tables for nested definitions. Symbol names begin with `@`.
- Symbols are unique within the defining module. Shadowing is forbidden.
- Imported or forward-declared symbols appear in the symbol table with the state `Decl`.
- Each module declares one or more dialect requirements via `air.requires`.

## Regions, Blocks, Values

- Regions are ordered lists of blocks. The entry region has no arguments.
- Blocks are introduced with `^label` and may take block arguments, each of which is an SSA value.
- SSA values begin with `%` and are defined exactly once.
- Resource values are SSA values tagged with resource type classes. A resource definition must be consumed or returned along every path unless explicitly released.

## Operations

Every operation records the following fields:

| Field | Description |
| --- | --- |
| name | Fully qualified operation identifier (`air.add`) |
| operands | Ordered list of SSA or resource values |
| results | Ordered list of SSA or resource values, each with a type |
| regions | Nested regions |
| successors | Control-flow transfer targets |
| attributes | Typed key-value tuples |
| types | Result type declarations and function types |
| effects | Declared effect set from the effect lattice |
| resource reads/writes | Explicit read/write set per resource |
| authority requirements | Required authority tokens |
| failure modes | Enumerated failure identifiers |
| rollback | Compensation contract or `none` |
| provenance | Proof or trace pointer for auditing |
| trust | Trust label for produced values |
| lowering contract | Dialect lowering obligations |
| optimization contract | Rewrites that preserve semantics |

Operations may not hide semantics; all behavior must be recoverable from the declared fields.

## Attributes

Attributes provide structured metadata. They are immutable, serializable values drawn from the type system.
Attributes are single-assignment and may not contain executable code.

## Location Information

Every IR node carries a `Location` comprising the primary source span and optional call-site chain.
Locations are required for diagnostics, trace correlation, and provenance.

## Textual Syntax

```
module      ::= `air.module` symbol `{` region `}`
region      ::= block*
block       ::= `^` ident? block-args? `:` op+
block-args  ::= `(` arg (`,` arg)* `)`
arg         ::= value-id `:` type
op          ::= result-binding? op-name operand-list? attr-list? type-annotation? effect-annotation? newline
result-binding ::= value-id (`=` | `,` value-id )* `=`
operand-list    ::= operand (`,` operand)*
operand         ::= value-id | symbol-ref | literal
type-annotation ::= `:` function-type | value-type
effect-annotation ::= `effects` `[` effect (`,` effect)* `]`
```

The grammar is intentionally LR-friendly for parser generation and hand-written recursive descent.

## Diagnostics

- Diagnostics reference the offending source span and a severity (`error`, `warning`, `note`).
- Diagnostics must not panic the parser or verifier.
- Suggested fixes use source spans and replacement text.

## Verification Phases

1. **Parse Check:** syntactic well-formedness, reserved keyword use.
2. **SSA Check:** definition-before-use, dominance, block terminators.
3. **Type Check:** operand/result type matching, dependent index constraints.
4. **Resource Check:** linear usage of resources, capability scoping.
5. **Effect Check:** effect declarations align with operation semantics.
6. **Authority Check:** required authority tokens are available and valid.
7. **Policy Check:** policy obligations are proven or surfaced as runtime checks.
8. **Provenance Check:** every effectful op has provenance metadata.

All phases run on every module before execution or transformation.

## Minimal Well-Formed Example

```
air.module @example {
  air.func @add(%a: i64, %b: i64) -> i64 {
  ^entry:
    %c = air.add %a, %b : i64
    air.return %c : i64
  }
}
```

## Failure Conventions

- Operations that may fail must enumerate failure modes.
- Verification ensures callers either handle failures or the operation is marked `may_trap`.
- Rollback behavior is explicit; absence of rollback defaults to atomic failure.

## Provenance and Trust

- Every produced value with trust lower than `trusted` must carry provenance describing origin.
- Trust levels form a lattice (`trusted >= verified >= model_generated >= third_party`).
- Policy checks may require proof that trust has not been escalated without justification.

## Contracts for Transformations

- Transformations must declare input/output dialects and preserved analyses.
- Every transformation must re-run verification for affected modules.
- Transformations that erase proofs must insert equivalent runtime checks.

## Traceability

- Every effectful operation emits a trace event keyed by the operation location, authority token, and effect set.
- Deterministic replay requires that trace events capture nondeterministic inputs (e.g., RNG values).

## Deterministic Replay Hooks

- Modules may declare `air.replay.hint` attributes to indicate state capture boundaries.
- Interpreter and runtime honor these hints to ensure reproducible execution.

## Extensibility

- Dialects may extend the operation set but must publish their semantic model.
- Dialects register with the module context and provide verifier hooks.
- Dialects may depend on other dialects; cyclic dependencies are disallowed.
