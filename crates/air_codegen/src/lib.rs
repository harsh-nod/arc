use air_ir::Module;

pub fn codegen_stub(module: &Module, target: &str) -> String {
    format!(
        "Code generation for {} not yet implemented (target {}).",
        module.name, target
    )
}
