# AIR Provenance Model

## Provenance Records

- Each value may attach `#air.prov` metadata describing origin (operation, authority, policy).
- Provenance forms a DAG enabling audits and replay alignment.

## Requirements

- Effectful operations must emit provenance for outputs and state transitions.
- Values derived from untrusted sources carry trust labels referencing provenance nodes.
- Transformations preserve provenance or record erasure with justification.

## Trace Correlation

- Trace events cite provenance IDs so external observers can reconstruct causality.
- Provenance ties into diagnostic explanations via `airc explain`.
