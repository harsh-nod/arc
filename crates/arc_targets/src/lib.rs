/// Target machine description.
#[derive(Debug, Clone)]
pub struct TargetDescription {
    pub name: &'static str,
    pub pointer_size: u8,
    pub registers: &'static [RegInfo],
    pub calling_convention: CallingConvention,
}

/// Information about a physical register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegInfo {
    pub name: &'static str,
    pub index: u8,
    /// Hardware encoding for instruction emission.
    pub hw_enc: u8,
    pub class: RegClass,
    /// Can the register allocator use this register?
    pub allocatable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegClass {
    Gpr,
    Fp,
}

/// Calling convention specifying where arguments and return values go.
#[derive(Debug, Clone)]
pub struct CallingConvention {
    /// Registers used for integer arguments, in order.
    pub int_arg_regs: &'static [u8],
    /// Register used for integer return value.
    pub int_ret_reg: u8,
    /// Callee-saved registers (must be preserved across calls).
    pub callee_saved: &'static [u8],
    /// Caller-saved registers (may be clobbered by calls).
    pub caller_saved: &'static [u8],
    /// Stack pointer register index.
    pub stack_pointer: u8,
    /// Frame pointer register index.
    pub frame_pointer: u8,
}

// ---------------------------------------------------------------------------
// x86_64 System V AMD64 ABI
// ---------------------------------------------------------------------------

pub mod x86_64 {
    use super::*;

    // Register indices (our internal numbering)
    pub const RAX: u8 = 0;
    pub const RCX: u8 = 1;
    pub const RDX: u8 = 2;
    pub const RBX: u8 = 3;
    pub const RSP: u8 = 4;
    pub const RBP: u8 = 5;
    pub const RSI: u8 = 6;
    pub const RDI: u8 = 7;
    pub const R8: u8 = 8;
    pub const R9: u8 = 9;
    pub const R10: u8 = 10;
    pub const R11: u8 = 11;
    pub const R12: u8 = 12;
    pub const R13: u8 = 13;
    pub const R14: u8 = 14;
    pub const R15: u8 = 15;

    pub static REGISTERS: &[RegInfo] = &[
        RegInfo {
            name: "rax",
            index: RAX,
            hw_enc: 0,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "rcx",
            index: RCX,
            hw_enc: 1,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "rdx",
            index: RDX,
            hw_enc: 2,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "rbx",
            index: RBX,
            hw_enc: 3,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "rsp",
            index: RSP,
            hw_enc: 4,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "rbp",
            index: RBP,
            hw_enc: 5,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "rsi",
            index: RSI,
            hw_enc: 6,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "rdi",
            index: RDI,
            hw_enc: 7,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "r8",
            index: R8,
            hw_enc: 0,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "r9",
            index: R9,
            hw_enc: 1,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "r10",
            index: R10,
            hw_enc: 2,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "r11",
            index: R11,
            hw_enc: 3,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "r12",
            index: R12,
            hw_enc: 4,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "r13",
            index: R13,
            hw_enc: 5,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "r14",
            index: R14,
            hw_enc: 6,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "r15",
            index: R15,
            hw_enc: 7,
            class: RegClass::Gpr,
            allocatable: true,
        },
    ];

    /// System V AMD64 calling convention.
    pub static SYSV_CC: CallingConvention = CallingConvention {
        int_arg_regs: &[RDI, RSI, RDX, RCX, R8, R9],
        int_ret_reg: RAX,
        callee_saved: &[RBX, RBP, R12, R13, R14, R15],
        caller_saved: &[RAX, RCX, RDX, RSI, RDI, R8, R9, R10, R11],
        stack_pointer: RSP,
        frame_pointer: RBP,
    };

    pub fn target() -> TargetDescription {
        TargetDescription {
            name: "x86_64-arc-linux-elf",
            pointer_size: 8,
            registers: REGISTERS,
            calling_convention: SYSV_CC.clone(),
        }
    }

    /// Returns true if register index >= 8 (needs REX prefix).
    pub fn needs_rex_ext(reg_index: u8) -> bool {
        reg_index >= 8
    }
}

// ---------------------------------------------------------------------------
// WebAssembly 32-bit target
// ---------------------------------------------------------------------------

pub mod wasm32 {
    use super::*;

    // Wasm uses a stack machine with local variables, not physical registers.
    // We model locals as virtual "registers" for the AIR pipeline.
    pub const LOCAL0: u8 = 0;
    pub const LOCAL1: u8 = 1;
    pub const LOCAL2: u8 = 2;
    pub const LOCAL3: u8 = 3;
    pub const LOCAL4: u8 = 4;
    pub const LOCAL5: u8 = 5;
    pub const LOCAL6: u8 = 6;
    pub const LOCAL7: u8 = 7;

    pub static LOCALS: &[RegInfo] = &[
        RegInfo {
            name: "local0",
            index: LOCAL0,
            hw_enc: 0,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "local1",
            index: LOCAL1,
            hw_enc: 1,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "local2",
            index: LOCAL2,
            hw_enc: 2,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "local3",
            index: LOCAL3,
            hw_enc: 3,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "local4",
            index: LOCAL4,
            hw_enc: 4,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "local5",
            index: LOCAL5,
            hw_enc: 5,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "local6",
            index: LOCAL6,
            hw_enc: 6,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "local7",
            index: LOCAL7,
            hw_enc: 7,
            class: RegClass::Gpr,
            allocatable: true,
        },
    ];

    /// Wasm calling convention: arguments in locals 0..N, return via stack.
    pub static WASM_CC: CallingConvention = CallingConvention {
        int_arg_regs: &[LOCAL0, LOCAL1, LOCAL2, LOCAL3, LOCAL4, LOCAL5],
        int_ret_reg: LOCAL0, // conceptual — wasm returns via stack
        callee_saved: &[],   // no callee-saved in wasm
        caller_saved: &[
            LOCAL0, LOCAL1, LOCAL2, LOCAL3, LOCAL4, LOCAL5, LOCAL6, LOCAL7,
        ],
        stack_pointer: LOCAL7, // not really used
        frame_pointer: LOCAL6, // not really used
    };

    pub fn target() -> TargetDescription {
        TargetDescription {
            name: "wasm32-arc",
            pointer_size: 4,
            registers: LOCALS,
            calling_convention: WASM_CC.clone(),
        }
    }

    // Wasm binary encoding opcodes (subset).
    pub const OP_UNREACHABLE: u8 = 0x00;
    pub const OP_NOP: u8 = 0x01;
    pub const OP_BLOCK: u8 = 0x02;
    pub const OP_LOOP: u8 = 0x03;
    pub const OP_IF: u8 = 0x04;
    pub const OP_ELSE: u8 = 0x05;
    pub const OP_END: u8 = 0x0B;
    pub const OP_BR: u8 = 0x0C;
    pub const OP_BR_IF: u8 = 0x0D;
    pub const OP_RETURN: u8 = 0x0F;
    pub const OP_CALL: u8 = 0x10;
    pub const OP_LOCAL_GET: u8 = 0x20;
    pub const OP_LOCAL_SET: u8 = 0x21;
    pub const OP_I32_CONST: u8 = 0x41;
    pub const OP_I64_CONST: u8 = 0x42;
    pub const OP_I64_ADD: u8 = 0x7C;
    pub const OP_I64_SUB: u8 = 0x7D;
    pub const OP_I64_MUL: u8 = 0x7E;
    pub const OP_I64_DIV_S: u8 = 0x7F;
    pub const OP_I64_EQ: u8 = 0x51;
    pub const OP_I64_NE: u8 = 0x52;
    pub const OP_I64_LT_S: u8 = 0x53;
    pub const OP_I64_GT_S: u8 = 0x55;
    pub const OP_I64_LE_S: u8 = 0x57;
    pub const OP_I64_GE_S: u8 = 0x59;

    /// Encode a signed LEB128 integer.
    pub fn encode_sleb128(mut value: i64) -> Vec<u8> {
        let mut result = Vec::new();
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            let more = !((value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0));
            if more {
                result.push(byte | 0x80);
            } else {
                result.push(byte);
                break;
            }
        }
        result
    }

    /// Encode an unsigned LEB128 integer.
    pub fn encode_uleb128(mut value: u64) -> Vec<u8> {
        let mut result = Vec::new();
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                result.push(byte | 0x80);
            } else {
                result.push(byte);
                break;
            }
        }
        result
    }
}

// ---------------------------------------------------------------------------
// AArch64 (ARM64) target — AAPCS64 calling convention
// ---------------------------------------------------------------------------

pub mod aarch64 {
    use super::*;

    // General-purpose registers x0–x30, sp (x31 alias), xzr
    pub const X0: u8 = 0;
    pub const X1: u8 = 1;
    pub const X2: u8 = 2;
    pub const X3: u8 = 3;
    pub const X4: u8 = 4;
    pub const X5: u8 = 5;
    pub const X6: u8 = 6;
    pub const X7: u8 = 7;
    pub const X8: u8 = 8; // indirect result location
    pub const X9: u8 = 9;
    pub const X10: u8 = 10;
    pub const X11: u8 = 11;
    pub const X12: u8 = 12;
    pub const X13: u8 = 13;
    pub const X14: u8 = 14;
    pub const X15: u8 = 15;
    pub const X16: u8 = 16; // IP0 (intra-procedure scratch)
    pub const X17: u8 = 17; // IP1
    pub const X18: u8 = 18; // platform register (reserved)
    pub const X19: u8 = 19;
    pub const X20: u8 = 20;
    pub const X21: u8 = 21;
    pub const X22: u8 = 22;
    pub const X23: u8 = 23;
    pub const X24: u8 = 24;
    pub const X25: u8 = 25;
    pub const X26: u8 = 26;
    pub const X27: u8 = 27;
    pub const X28: u8 = 28;
    pub const FP: u8 = 29; // frame pointer (x29)
    pub const LR: u8 = 30; // link register (x30)
    pub const SP: u8 = 31; // stack pointer

    pub static REGISTERS: &[RegInfo] = &[
        RegInfo {
            name: "x0",
            index: X0,
            hw_enc: 0,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x1",
            index: X1,
            hw_enc: 1,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x2",
            index: X2,
            hw_enc: 2,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x3",
            index: X3,
            hw_enc: 3,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x4",
            index: X4,
            hw_enc: 4,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x5",
            index: X5,
            hw_enc: 5,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x6",
            index: X6,
            hw_enc: 6,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x7",
            index: X7,
            hw_enc: 7,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x8",
            index: X8,
            hw_enc: 8,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x9",
            index: X9,
            hw_enc: 9,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x10",
            index: X10,
            hw_enc: 10,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x11",
            index: X11,
            hw_enc: 11,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x12",
            index: X12,
            hw_enc: 12,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x13",
            index: X13,
            hw_enc: 13,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x14",
            index: X14,
            hw_enc: 14,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x15",
            index: X15,
            hw_enc: 15,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x16",
            index: X16,
            hw_enc: 16,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "x17",
            index: X17,
            hw_enc: 17,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "x18",
            index: X18,
            hw_enc: 18,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "x19",
            index: X19,
            hw_enc: 19,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x20",
            index: X20,
            hw_enc: 20,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x21",
            index: X21,
            hw_enc: 21,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x22",
            index: X22,
            hw_enc: 22,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x23",
            index: X23,
            hw_enc: 23,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x24",
            index: X24,
            hw_enc: 24,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x25",
            index: X25,
            hw_enc: 25,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x26",
            index: X26,
            hw_enc: 26,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x27",
            index: X27,
            hw_enc: 27,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "x28",
            index: X28,
            hw_enc: 28,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "fp",
            index: FP,
            hw_enc: 29,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "lr",
            index: LR,
            hw_enc: 30,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "sp",
            index: SP,
            hw_enc: 31,
            class: RegClass::Gpr,
            allocatable: false,
        },
    ];

    /// AAPCS64 calling convention.
    pub static AAPCS64_CC: CallingConvention = CallingConvention {
        int_arg_regs: &[X0, X1, X2, X3, X4, X5, X6, X7],
        int_ret_reg: X0,
        callee_saved: &[X19, X20, X21, X22, X23, X24, X25, X26, X27, X28, FP, LR],
        caller_saved: &[
            X0, X1, X2, X3, X4, X5, X6, X7, X8, X9, X10, X11, X12, X13, X14, X15, X16, X17,
        ],
        stack_pointer: SP,
        frame_pointer: FP,
    };

    pub fn target() -> TargetDescription {
        TargetDescription {
            name: "aarch64-air-linux-elf",
            pointer_size: 8,
            registers: REGISTERS,
            calling_convention: AAPCS64_CC.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// RISC-V 64-bit target (RV64I) — standard RISC-V calling convention
// ---------------------------------------------------------------------------

pub mod riscv64 {
    use super::*;

    // Integer registers x0–x31
    pub const ZERO: u8 = 0; // x0 — hardwired zero
    pub const RA: u8 = 1; // x1 — return address
    pub const SP: u8 = 2; // x2 — stack pointer
    pub const GP: u8 = 3; // x3 — global pointer
    pub const TP: u8 = 4; // x4 — thread pointer
    pub const T0: u8 = 5; // x5 — temp
    pub const T1: u8 = 6; // x6
    pub const T2: u8 = 7; // x7
    pub const S0: u8 = 8; // x8 — frame pointer / saved
    pub const S1: u8 = 9; // x9
    pub const A0: u8 = 10; // x10 — argument / return value
    pub const A1: u8 = 11; // x11 — argument / return value
    pub const A2: u8 = 12; // x12
    pub const A3: u8 = 13; // x13
    pub const A4: u8 = 14; // x14
    pub const A5: u8 = 15; // x15
    pub const A6: u8 = 16; // x16
    pub const A7: u8 = 17; // x17
    pub const S2: u8 = 18; // x18
    pub const S3: u8 = 19; // x19
    pub const S4: u8 = 20; // x20
    pub const S5: u8 = 21; // x21
    pub const S6: u8 = 22; // x22
    pub const S7: u8 = 23; // x23
    pub const S8: u8 = 24; // x24
    pub const S9: u8 = 25; // x25
    pub const S10: u8 = 26; // x26
    pub const S11: u8 = 27; // x27
    pub const T3: u8 = 28; // x28
    pub const T4: u8 = 29; // x29
    pub const T5: u8 = 30; // x30
    pub const T6: u8 = 31; // x31

    pub static REGISTERS: &[RegInfo] = &[
        RegInfo {
            name: "zero",
            index: ZERO,
            hw_enc: 0,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "ra",
            index: RA,
            hw_enc: 1,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "sp",
            index: SP,
            hw_enc: 2,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "gp",
            index: GP,
            hw_enc: 3,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "tp",
            index: TP,
            hw_enc: 4,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "t0",
            index: T0,
            hw_enc: 5,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "t1",
            index: T1,
            hw_enc: 6,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "t2",
            index: T2,
            hw_enc: 7,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "s0",
            index: S0,
            hw_enc: 8,
            class: RegClass::Gpr,
            allocatable: false,
        },
        RegInfo {
            name: "s1",
            index: S1,
            hw_enc: 9,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "a0",
            index: A0,
            hw_enc: 10,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "a1",
            index: A1,
            hw_enc: 11,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "a2",
            index: A2,
            hw_enc: 12,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "a3",
            index: A3,
            hw_enc: 13,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "a4",
            index: A4,
            hw_enc: 14,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "a5",
            index: A5,
            hw_enc: 15,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "a6",
            index: A6,
            hw_enc: 16,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "a7",
            index: A7,
            hw_enc: 17,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "s2",
            index: S2,
            hw_enc: 18,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "s3",
            index: S3,
            hw_enc: 19,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "s4",
            index: S4,
            hw_enc: 20,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "s5",
            index: S5,
            hw_enc: 21,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "s6",
            index: S6,
            hw_enc: 22,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "s7",
            index: S7,
            hw_enc: 23,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "s8",
            index: S8,
            hw_enc: 24,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "s9",
            index: S9,
            hw_enc: 25,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "s10",
            index: S10,
            hw_enc: 26,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "s11",
            index: S11,
            hw_enc: 27,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "t3",
            index: T3,
            hw_enc: 28,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "t4",
            index: T4,
            hw_enc: 29,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "t5",
            index: T5,
            hw_enc: 30,
            class: RegClass::Gpr,
            allocatable: true,
        },
        RegInfo {
            name: "t6",
            index: T6,
            hw_enc: 31,
            class: RegClass::Gpr,
            allocatable: true,
        },
    ];

    /// Standard RISC-V calling convention.
    pub static RV64_CC: CallingConvention = CallingConvention {
        int_arg_regs: &[A0, A1, A2, A3, A4, A5, A6, A7],
        int_ret_reg: A0,
        callee_saved: &[S0, S1, S2, S3, S4, S5, S6, S7, S8, S9, S10, S11],
        caller_saved: &[T0, T1, T2, T3, T4, T5, T6, A0, A1, A2, A3, A4, A5, A6, A7],
        stack_pointer: SP,
        frame_pointer: S0,
    };

    pub fn target() -> TargetDescription {
        TargetDescription {
            name: "riscv64-air-linux-elf",
            pointer_size: 8,
            registers: REGISTERS,
            calling_convention: RV64_CC.clone(),
        }
    }
}

pub fn builtin_targets() -> Vec<TargetDescription> {
    vec![
        x86_64::target(),
        wasm32::target(),
        aarch64::target(),
        riscv64::target(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x86_64_has_16_registers() {
        assert_eq!(x86_64::REGISTERS.len(), 16);
    }

    #[test]
    fn rsp_rbp_not_allocatable() {
        let rsp = &x86_64::REGISTERS[x86_64::RSP as usize];
        let rbp = &x86_64::REGISTERS[x86_64::RBP as usize];
        assert!(!rsp.allocatable);
        assert!(!rbp.allocatable);
    }

    #[test]
    fn sysv_arg_regs_order() {
        let cc = &x86_64::SYSV_CC;
        assert_eq!(cc.int_arg_regs[0], x86_64::RDI);
        assert_eq!(cc.int_arg_regs[1], x86_64::RSI);
        assert_eq!(cc.int_ret_reg, x86_64::RAX);
    }

    #[test]
    fn rex_ext_detection() {
        assert!(!x86_64::needs_rex_ext(x86_64::RAX));
        assert!(x86_64::needs_rex_ext(x86_64::R8));
        assert!(x86_64::needs_rex_ext(x86_64::R15));
    }

    #[test]
    fn wasm32_has_8_locals() {
        assert_eq!(wasm32::LOCALS.len(), 8);
    }

    #[test]
    fn wasm32_all_allocatable() {
        assert!(wasm32::LOCALS.iter().all(|r| r.allocatable));
    }

    #[test]
    fn wasm32_pointer_size() {
        let target = wasm32::target();
        assert_eq!(target.pointer_size, 4);
    }

    #[test]
    fn builtin_targets_has_all() {
        let targets = builtin_targets();
        assert_eq!(targets.len(), 4);
        assert_eq!(targets[0].name, "x86_64-arc-linux-elf");
        assert_eq!(targets[1].name, "wasm32-arc");
        assert_eq!(targets[2].name, "aarch64-air-linux-elf");
        assert_eq!(targets[3].name, "riscv64-air-linux-elf");
    }

    #[test]
    fn aarch64_has_32_registers() {
        assert_eq!(aarch64::REGISTERS.len(), 32);
    }

    #[test]
    fn aarch64_sp_fp_not_allocatable() {
        let sp = &aarch64::REGISTERS[aarch64::SP as usize];
        let fp = &aarch64::REGISTERS[aarch64::FP as usize];
        assert!(!sp.allocatable);
        assert!(!fp.allocatable);
    }

    #[test]
    fn aarch64_arg_regs() {
        let cc = &aarch64::AAPCS64_CC;
        assert_eq!(cc.int_arg_regs.len(), 8);
        assert_eq!(cc.int_arg_regs[0], aarch64::X0);
        assert_eq!(cc.int_ret_reg, aarch64::X0);
    }

    #[test]
    fn riscv64_has_32_registers() {
        assert_eq!(riscv64::REGISTERS.len(), 32);
    }

    #[test]
    fn riscv64_zero_not_allocatable() {
        let zero = &riscv64::REGISTERS[riscv64::ZERO as usize];
        assert!(!zero.allocatable);
    }

    #[test]
    fn riscv64_arg_regs() {
        let cc = &riscv64::RV64_CC;
        assert_eq!(cc.int_arg_regs.len(), 8);
        assert_eq!(cc.int_arg_regs[0], riscv64::A0);
        assert_eq!(cc.int_ret_reg, riscv64::A0);
    }

    #[test]
    fn leb128_encoding() {
        // 0 -> [0x00]
        assert_eq!(wasm32::encode_uleb128(0), vec![0x00]);
        // 127 -> [0x7f]
        assert_eq!(wasm32::encode_uleb128(127), vec![0x7f]);
        // 128 -> [0x80, 0x01]
        assert_eq!(wasm32::encode_uleb128(128), vec![0x80, 0x01]);
        // signed -1 -> [0x7f]
        assert_eq!(wasm32::encode_sleb128(-1), vec![0x7f]);
        // signed 0 -> [0x00]
        assert_eq!(wasm32::encode_sleb128(0), vec![0x00]);
        // signed 42 -> [0x2a]
        assert_eq!(wasm32::encode_sleb128(42), vec![0x2a]);
    }
}
