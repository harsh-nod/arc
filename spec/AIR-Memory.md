# AIR Memory Model

## Goals

- Provide a deterministic, capability-aware memory model suitable for interpreted and native execution.
- Support linear memory resources with explicit allocation and deallocation.
- Track provenance for memory accesses and pointer creation.

## Memory Resources

- `!air.mem`: linear heap state updated by allocate/deallocate/store operations.
- `!air.ptr<T, qualifiers>`: typed pointer with qualifiers such as `align`, `addrspace`, `mut`, `region`.
- `!air.slice<T, %n>`: view into contiguous memory with index `n`.

## Allocation

```
%mem1, %ptr = air.alloc %mem0, %size
  effects [#air.effect<allocate>]
  : (!air.mem, index) -> (!air.mem, !air.ptr<i8>)
```

- Allocation consumes the input memory resource and produces an updated resource plus pointer.
- Allocation failure must be reported via declared failure mode.

## Load and Store

```
%val = air.load %mem, %ptr
  effects [#air.effect<memory.read>]
  : (!air.mem, !air.ptr<T>) -> T
```

```
%mem1 = air.store %mem0, %ptr, %val
  effects [#air.effect<memory.write>]
  : (!air.mem, !air.ptr<T>, T) -> !air.mem
```

- Loads require proof of bounds or rely on runtime checks inserted by verifier.
- Stores consume the incoming memory resource and return the updated resource.

## Aliasing

- AIR assumes no hidden aliasing. To create aliases, use explicit `air.alias` op that records provenance.
- Aliases must obey linearity; once an alias is consumed, it cannot be reused.

## Provenance Tracking

- Every pointer remembers allocation site, authority, and policy.
- Provenance feeds into the verifier to ensure safe deallocation and policy enforcement.

## Deallocation

```
%mem1 = air.dealloc %mem0, %ptr
  effects [#air.effect<deallocate>]
```

- Deallocation requires that `%ptr` be live and unique.
- Double free is detected by resource verifier.

## Memory in the Interpreter

- Modeled using persistent data structures for deterministic replay.
- Addresses are abstract handles rather than raw integers.
- Traces record allocation IDs to align interpreter and backend runs.
