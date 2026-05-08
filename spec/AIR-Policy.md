# AIR Policy System

## Policy Types

- `!air.policy<name = @policy.require_approval>`
- Policies generate proof obligations and runtime checks.

## Enforcement Points

- `air.policy.check` produces proofs when policy conditions are met.
- `air.policy.enforce` inserts runtime enforcement, trapping or emitting failure events.
- Policies may declassify data, grant authorities, or restrict capabilities.

## Proof Obligations

- Policy proofs must dominate the use of restricted operations.
- Revocation invalidates existing proofs; verifier ensures revalidation.
- Policies integrate with effect system so that dangerous effects require policy justification.

## Trace Integration

- Entry, approval, revocation, and enforcement events are recorded for audit.
- Replay confirms that policy outcomes are deterministic under the recorded approvals.
