# AIR Object and Packaging Model

## File Formats

- `.air`: textual IR.
- `.airb`: binary IR with LEB128 and string tables.
- `.airtrace`: textual or binary traces.
- `.airpkg`: package archives containing modules, policies, capabilities, proofs, targets.
- `.airo`: AIR object files representing compiled artifacts or wrappers around native objects.

## Object Layout

- Sections: `.air.text`, `.air.data`, `.air.proof`, `.air.trust`, `.air.meta`.
- Symbols: exported functions, capabilities, policies, data.
- Relocations: reference table linking targets to symbol offsets.

## Package Manifest

```
air.package @report_agent {
  modules [@main]
  requires [
    @capability.email.compose,
    @capability.email.send
  ]
  policies [
    @policy.require_approval_for_external_send
  ]
  targets [
    @x86_64_air_linux,
    @wasm32_air,
    @air_runtime
  ]
}
```

## Validation

- Hash-based content addressing ensures integrity.
- Signatures attach to package manifests.
- Capability manifests checked before execution.

## Versioning

- Packages embed semantic version and dialect revision numbers.
- Loader rejects incompatible dialect versions unless migration is provided.
