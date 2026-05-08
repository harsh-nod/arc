# AIR Lowering Contracts

## Multi-Level Representation

1. **AIR-HL:** high-level dialects (agent, policy, tensor) with structured control flow.
2. **AIR-CFG:** canonical control-flow graph with blocks and branches.
3. **AIR-M:** machine-independent low-level operations.
4. **AIR-Target:** target-specific virtual register form.
5. **AIR-Phys:** physical register and stack layout.
6. **AIR-Obj:** object emission model.

## Contracts

- Every lowering step must declare preserved properties: return values, effects, authorities, policies, provenance.
- Proof obligations attach to lowering definitions. If unsatisfied, the lowering must insert runtime checks.
- Lowering passes run verifier after transformation.

## Trace Equivalence

- Lowering preserves observable trace modulo target-specific instrumentation.
- `airc trace-compare` verifies equivalence between levels.
