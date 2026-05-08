# AIR Trace Format

## Purpose

Traces capture effectful execution for auditing, replay, and explanation.

## Structure

```
air.trace @run_001 {
  air.event @e1 invoke @main
  air.event @e2 memory.alloc %buf
  air.event @e3 proof.checked %pf
  air.event @e4 capability.proposed @email.send
  air.event @e5 human.approved %auth
  air.event @e6 capability.invoked @email.send
  air.event @e7 returned @main
}
```

- Events carry timestamp, effect set, authority tokens, provenance references, and trust labels.
- Non-deterministic inputs (RNG, timestamps) record concrete values for replay.

## Serialization

- Textual `.airtrace` files follow the syntax above.
- Binary form packs events with LEB128 encoding for compact storage.

## Replay

- `airc replay` uses traces to drive deterministic execution.
- Replay checks that runtime emits identical events; divergence raises diagnostics.
