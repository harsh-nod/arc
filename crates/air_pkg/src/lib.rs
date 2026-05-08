use air_ir::Symbol;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageManifest {
    pub name: Symbol,
    pub modules: Vec<Symbol>,
}

impl PackageManifest {
    pub fn new(name: Symbol, modules: Vec<Symbol>) -> Self {
        Self { name, modules }
    }
}
