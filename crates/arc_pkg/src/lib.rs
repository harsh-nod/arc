//! AIR package format: serialization, manifests, and dependency resolution.

use arc_ir::Module;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A package manifest describing an AIR package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    pub modules: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub description: String,
}

impl PackageManifest {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            modules: Vec::new(),
            dependencies: Vec::new(),
            description: String::new(),
        }
    }

    pub fn add_module(&mut self, module_path: impl Into<String>) {
        self.modules.push(module_path.into());
    }

    pub fn add_dependency(&mut self, dep: Dependency) {
        self.dependencies.push(dep);
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Write manifest to a file.
    pub fn write_to_file(&self, path: &Path) -> Result<(), PackageError> {
        let json = self
            .to_json()
            .map_err(|e| PackageError::Serialization(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| PackageError::Io(e.to_string()))
    }

    /// Read manifest from a file.
    pub fn read_from_file(path: &Path) -> Result<Self, PackageError> {
        let json = std::fs::read_to_string(path).map_err(|e| PackageError::Io(e.to_string()))?;
        Self::from_json(&json).map_err(|e| PackageError::Serialization(e.to_string()))
    }
}

/// A dependency on another package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub path: Option<String>,
}

impl Dependency {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            path: None,
        }
    }

    pub fn local(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: "0.0.0".to_string(),
            path: Some(path.into()),
        }
    }
}

/// A serialized AIR module (IR as JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedModule {
    pub module: Module,
}

impl SerializedModule {
    pub fn new(module: Module) -> Self {
        Self { module }
    }

    /// Serialize a module to JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize a module from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// A loaded package with resolved modules.
#[derive(Debug)]
pub struct Package {
    pub manifest: PackageManifest,
    pub modules: HashMap<String, Module>,
    pub root: PathBuf,
}

impl Package {
    /// Load a package from a directory containing an `arc-pkg.json` manifest.
    pub fn load(dir: &Path) -> Result<Self, PackageError> {
        let manifest_path = dir.join("arc-pkg.json");
        let manifest = PackageManifest::read_from_file(&manifest_path)?;

        let mut modules = HashMap::new();
        for module_path in &manifest.modules {
            let full_path = dir.join(module_path);
            let source = std::fs::read_to_string(&full_path)
                .map_err(|e| PackageError::Io(format!("{}: {}", full_path.display(), e)))?;
            let module = arc_syntax::parse_module(&source)
                .map_err(|e| PackageError::Parse(format!("{}: {}", module_path, e)))?;
            modules.insert(module_path.clone(), module);
        }

        Ok(Package {
            manifest,
            modules,
            root: dir.to_path_buf(),
        })
    }

    /// Get a module by its path within the package.
    pub fn get_module(&self, path: &str) -> Option<&Module> {
        self.modules.get(path)
    }

    /// List all module names in this package.
    pub fn module_names(&self) -> Vec<&str> {
        self.modules.keys().map(|s| s.as_str()).collect()
    }
}

/// Topological sort of dependencies.
/// Returns package names in build order (dependencies before dependents).
pub fn resolve_build_order(
    packages: &HashMap<String, PackageManifest>,
) -> Result<Vec<String>, PackageError> {
    let mut visited: HashMap<String, bool> = HashMap::new(); // true = permanent, false = temporary
    let mut order = Vec::new();

    for name in packages.keys() {
        if !visited.contains_key(name) {
            topo_visit(name, packages, &mut visited, &mut order)?;
        }
    }

    Ok(order)
}

fn topo_visit(
    name: &str,
    packages: &HashMap<String, PackageManifest>,
    visited: &mut HashMap<String, bool>,
    order: &mut Vec<String>,
) -> Result<(), PackageError> {
    if let Some(&permanent) = visited.get(name) {
        if permanent {
            return Ok(());
        } else {
            return Err(PackageError::CyclicDependency(name.to_string()));
        }
    }

    visited.insert(name.to_string(), false); // temporary mark

    if let Some(pkg) = packages.get(name) {
        for dep in &pkg.dependencies {
            topo_visit(&dep.name, packages, visited, order)?;
        }
    }

    visited.insert(name.to_string(), true); // permanent mark
    order.push(name.to_string());
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("IO error: {0}")]
    Io(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("cyclic dependency involving {0}")]
    CyclicDependency(String),
    #[error("missing dependency: {0}")]
    MissingDependency(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_ir::Symbol;

    #[test]
    fn manifest_roundtrip_json() {
        let mut manifest = PackageManifest::new("my_pkg", "0.1.0");
        manifest.add_module("src/main.air".to_string());
        manifest.add_module("src/lib.air".to_string());
        manifest.add_dependency(Dependency::new("base", "1.0.0"));
        manifest.description = "A test package".to_string();

        let json = manifest.to_json().unwrap();
        let parsed = PackageManifest::from_json(&json).unwrap();

        assert_eq!(parsed.name, "my_pkg");
        assert_eq!(parsed.version, "0.1.0");
        assert_eq!(parsed.modules.len(), 2);
        assert_eq!(parsed.dependencies.len(), 1);
        assert_eq!(parsed.dependencies[0].name, "base");
    }

    #[test]
    fn module_serialization() {
        let module = Module::new(Symbol::new("test_module"));
        let serialized = SerializedModule::new(module);
        let json = serialized.to_json().unwrap();
        let deserialized = SerializedModule::from_json(&json).unwrap();
        assert_eq!(deserialized.module.name.as_str(), "test_module");
    }

    #[test]
    fn dependency_resolution_linear() {
        let mut packages = HashMap::new();

        let mut a = PackageManifest::new("a", "1.0.0");
        a.add_dependency(Dependency::new("b", "1.0.0"));
        packages.insert("a".to_string(), a);

        let mut b = PackageManifest::new("b", "1.0.0");
        b.add_dependency(Dependency::new("c", "1.0.0"));
        packages.insert("b".to_string(), b);

        let c = PackageManifest::new("c", "1.0.0");
        packages.insert("c".to_string(), c);

        let order = resolve_build_order(&packages).unwrap();
        assert_eq!(order, vec!["c", "b", "a"]);
    }

    #[test]
    fn dependency_resolution_diamond() {
        let mut packages = HashMap::new();

        let mut a = PackageManifest::new("a", "1.0.0");
        a.add_dependency(Dependency::new("b", "1.0.0"));
        a.add_dependency(Dependency::new("c", "1.0.0"));
        packages.insert("a".to_string(), a);

        let mut b = PackageManifest::new("b", "1.0.0");
        b.add_dependency(Dependency::new("d", "1.0.0"));
        packages.insert("b".to_string(), b);

        let mut c = PackageManifest::new("c", "1.0.0");
        c.add_dependency(Dependency::new("d", "1.0.0"));
        packages.insert("c".to_string(), c);

        let d = PackageManifest::new("d", "1.0.0");
        packages.insert("d".to_string(), d);

        let order = resolve_build_order(&packages).unwrap();
        // d must come before b and c, which must come before a
        let d_pos = order.iter().position(|x| x == "d").unwrap();
        let b_pos = order.iter().position(|x| x == "b").unwrap();
        let c_pos = order.iter().position(|x| x == "c").unwrap();
        let a_pos = order.iter().position(|x| x == "a").unwrap();
        assert!(d_pos < b_pos);
        assert!(d_pos < c_pos);
        assert!(b_pos < a_pos);
        assert!(c_pos < a_pos);
    }

    #[test]
    fn dependency_cycle_detected() {
        let mut packages = HashMap::new();

        let mut a = PackageManifest::new("a", "1.0.0");
        a.add_dependency(Dependency::new("b", "1.0.0"));
        packages.insert("a".to_string(), a);

        let mut b = PackageManifest::new("b", "1.0.0");
        b.add_dependency(Dependency::new("a", "1.0.0"));
        packages.insert("b".to_string(), b);

        let err = resolve_build_order(&packages).unwrap_err();
        assert!(matches!(err, PackageError::CyclicDependency(_)));
    }

    #[test]
    fn local_dependency() {
        let dep = Dependency::local("my_lib", "../my_lib");
        assert_eq!(dep.name, "my_lib");
        assert_eq!(dep.path, Some("../my_lib".to_string()));
    }
}
