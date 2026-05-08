use air_ir::Module;

pub struct Runtime {
    pub name: String,
}

impl Runtime {
    pub fn sandbox() -> Self {
        Self {
            name: "sandbox".into(),
        }
    }

    pub fn load_module(&self, _module: &Module) {
        // Placeholder no-op.
    }
}
