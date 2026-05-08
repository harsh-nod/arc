# AIR Target Descriptions

## Target Specification

Targets declare machine properties, legal types, register classes, instruction templates, and calling conventions.

```
air.target @x86_64_air_linux {
  pointer_size = 64
  endian = little
  object_format = elf
  calling_convention = @sysv_amd64

  registers {
    class @gpr64 = [rax, rbx, rcx, rdx, rsi, rdi, rbp, rsp, r8, r9, r10, r11, r12, r13, r14, r15]
  }

  legal_types [i1, i8, i16, i32, i64, f32, f64]

  instruction @ADD64rr {
    operands(%dst: @gpr64, %a: @gpr64, %b: @gpr64)
    pattern = air.add %a, %b : i64
    latency = 1
    encoding = 0x01 /r
  }
}
```

## Legalization

- Type legalization maps unsupported types to sequences of legal ones.
- Operation legalization provides lowering patterns when direct encoding is unavailable.

## Register Allocation

- Register classes define interference domains.
- Spills map to stack slots with alignment and lifetime metadata.

## Instruction Selection

- Uses pattern-matching with proof obligations ensuring effect and authority preservation.

## Target Features

- Feature flags gate instructions requiring hardware support (e.g., AVX).
- Verifier prevents selection of unavailable features without proof.
