---
layout: default
title: Package and Binary Format
---

# Package and Binary Format

ARC includes experimental crates for serializing modules and packaging compiled
artifacts. These pieces are useful for testing and design validation. They are
not yet a stable distribution ABI.

## Text Format

The `.air` text format is the source-of-truth format for examples and tests.
The parser and printer support stable round trips for the documented subset.

```bash
arcc parse examples/hello.air --print
```

## Binary Module Format

The `arc_format` crate supports:

- compact binary module encoding
- JSON-backed module encoding
- content hashes
- signed package envelopes
- tamper detection tests

The binary format currently uses ARC's internal module representation. It may
change before a stable package release.

## Package Manifests

The `arc_pkg` crate models:

- package manifests
- local dependencies
- dependency graph resolution
- cycle detection
- module serialization

This is intended to grow into a reproducible package format for ARC modules,
capability manifests, generated objects, signatures, and traces.

## Object Files

The `arc_object` crate emits a minimal x86-64 ELF relocatable object containing:

- `.text`
- `.symtab`
- `.strtab`
- `.shstrtab`

The object emitter is intentionally small and direct. It is suitable for tests
and prototype compilation, not yet a full linker replacement.

## WebAssembly

The WebAssembly emitter writes a minimal wasm module for the lowered numeric
subset. It is useful for validating the target-independent low IR.

```bash
arcc codegen examples/hello.air --target wasm32
```

## Stability

For the 0.1 prototype:

- `.air` examples are public and documented
- CLI commands are intended to remain usable
- binary/package encodings are experimental
- generated `.o` and `.wasm` files should not be committed

