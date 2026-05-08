use air_ir::Module;

pub trait Pass {
    fn name(&self) -> &str;
    fn run(&self, module: &mut Module);
}

pub struct PassManager {
    passes: Vec<Box<dyn Pass>>,
}

impl PassManager {
    pub fn new() -> Self {
        Self { passes: Vec::new() }
    }

    pub fn add_pass<P: Pass + 'static>(&mut self, pass: P) {
        self.passes.push(Box::new(pass));
    }

    pub fn run(&self, module: &mut Module) {
        for pass in &self.passes {
            pass.run(module);
        }
    }
}
