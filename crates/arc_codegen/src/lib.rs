pub mod emit;
pub mod isel;
pub mod low_ir;
pub mod regalloc;
pub mod wasm_emit;

use arc_ir::Module;
use arc_object::ObjectFile;
use arc_targets::TargetDescription;

#[derive(Debug, thiserror::Error)]
pub enum CodegenError {
    #[error("codegen error: {0}")]
    Failed(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

/// Compile an AIR module to an object file for the given target.
pub fn compile(module: &Module, target: &TargetDescription) -> Result<ObjectFile, CodegenError> {
    // Step 1: Lower AIR to machine-independent low IR
    let low_module = low_ir::lower_module(module)?;

    // Step 2: Instruction selection (low IR → target instructions with virtual registers)
    let mut machine_funcs = Vec::new();
    for func in &low_module.functions {
        let mfunc = isel::select_instructions(func, target)?;
        machine_funcs.push(mfunc);
    }

    // Step 3: Register allocation
    for mfunc in &mut machine_funcs {
        regalloc::allocate_registers(mfunc, target)?;
    }

    // Step 4: Emit machine code
    let obj = emit::emit_object(&machine_funcs, target)?;
    Ok(obj)
}
