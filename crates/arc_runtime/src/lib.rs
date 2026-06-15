use arc_ir::Module;
use std::collections::HashMap;

/// The result of invoking a capability provider.
#[derive(Debug, Clone)]
pub struct InvokeResult {
    /// Output values, one per capability output.
    pub outputs: Vec<i64>,
}

/// Trait for capability providers that handle `arc.invoke` at runtime.
pub trait CapabilityProvider {
    /// Execute the capability with the given input values.
    /// Returns output values matching the capability's declared outputs.
    fn invoke(&self, inputs: &[i64]) -> Result<InvokeResult, ProviderError>;

    /// The name of this provider (must match the capability name in the module).
    fn name(&self) -> &str;
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider error: {0}")]
    Execution(String),
    #[error("capability not found: {0}")]
    NotFound(String),
    #[error("argument count mismatch: expected {expected}, got {got}")]
    ArgMismatch { expected: usize, got: usize },
}

/// A fake capability provider that records all invocations for testing.
#[derive(Debug)]
pub struct FakeProvider {
    name: String,
    output_count: usize,
    invocations: std::cell::RefCell<Vec<Vec<i64>>>,
}

impl FakeProvider {
    pub fn new(name: impl Into<String>, output_count: usize) -> Self {
        Self {
            name: name.into(),
            output_count,
            invocations: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Returns all recorded invocations (each is a vec of input values).
    pub fn invocations(&self) -> Vec<Vec<i64>> {
        self.invocations.borrow().clone()
    }

    /// Number of times this provider was invoked.
    pub fn call_count(&self) -> usize {
        self.invocations.borrow().len()
    }
}

impl CapabilityProvider for FakeProvider {
    fn invoke(&self, inputs: &[i64]) -> Result<InvokeResult, ProviderError> {
        self.invocations.borrow_mut().push(inputs.to_vec());
        Ok(InvokeResult {
            outputs: vec![0; self.output_count],
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Policy for handling approval requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalPolicy {
    /// Always grant approval (for testing).
    AlwaysAllow,
    /// Always deny approval.
    AlwaysDeny,
}

/// The AIR runtime manages capability providers and executes modules.
pub struct Runtime {
    pub name: String,
    providers: HashMap<String, Box<dyn CapabilityProvider>>,
    approval_policy: ApprovalPolicy,
}

impl Runtime {
    pub fn sandbox() -> Self {
        Self {
            name: "sandbox".into(),
            providers: HashMap::new(),
            approval_policy: ApprovalPolicy::AlwaysAllow,
        }
    }

    pub fn with_approval_policy(mut self, policy: ApprovalPolicy) -> Self {
        self.approval_policy = policy;
        self
    }

    /// Register a capability provider.
    pub fn register_provider(&mut self, provider: Box<dyn CapabilityProvider>) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    /// Check if a provider is registered for the given capability.
    pub fn has_provider(&self, capability: &str) -> bool {
        self.providers.contains_key(capability)
    }

    /// Invoke a capability by name with the given inputs.
    pub fn invoke_capability(
        &self,
        capability: &str,
        inputs: &[i64],
    ) -> Result<InvokeResult, ProviderError> {
        let provider = self
            .providers
            .get(capability)
            .ok_or_else(|| ProviderError::NotFound(capability.to_string()))?;
        provider.invoke(inputs)
    }

    /// Check approval for an action.
    pub fn check_approval(&self) -> bool {
        match self.approval_policy {
            ApprovalPolicy::AlwaysAllow => true,
            ApprovalPolicy::AlwaysDeny => false,
        }
    }

    /// Validate that the runtime has providers for all capabilities declared in the module.
    pub fn validate_module(&self, module: &Module) -> Result<(), Vec<String>> {
        let missing: Vec<String> = module
            .capabilities
            .keys()
            .filter(|cap| !self.providers.contains_key(cap.as_str()))
            .map(|cap| cap.as_str().to_string())
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }

    /// Auto-register fake providers for all capabilities in a module.
    pub fn register_fakes_for_module(&mut self, module: &Module) {
        for (name, cap) in &module.capabilities {
            if !self.providers.contains_key(name.as_str()) {
                let fake = FakeProvider::new(name.as_str(), cap.outputs.len());
                self.providers
                    .insert(name.as_str().to_string(), Box::new(fake));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_ir::{Argument, Capability, Location, Module, Symbol, Type, ValueId};

    fn loc() -> Location {
        Location::new(0, 0)
    }

    fn make_test_module() -> Module {
        let mut module = Module::new(Symbol::new("test"));
        let cap = Capability {
            name: Symbol::new("email.send"),
            inputs: vec![
                Argument {
                    name: ValueId::new("to"),
                    ty: Type::new("i64"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("body"),
                    ty: Type::new("i64"),
                    location: loc(),
                },
            ],
            outputs: vec![Argument {
                name: ValueId::new("status"),
                ty: Type::new("i64"),
                location: loc(),
            }],
            effects: vec!["network".to_string()],
            failures: vec!["delivery_error".to_string()],
            location: loc(),
        };
        module.add_capability(cap).unwrap();
        module
    }

    #[test]
    fn fake_provider_records_invocations() {
        let provider = FakeProvider::new("email.send", 1);
        let result = provider.invoke(&[1, 2]).unwrap();
        assert_eq!(result.outputs.len(), 1);
        assert_eq!(provider.call_count(), 1);
        assert_eq!(provider.invocations()[0], vec![1, 2]);
    }

    #[test]
    fn runtime_dispatches_to_provider() {
        let mut runtime = Runtime::sandbox();
        runtime.register_provider(Box::new(FakeProvider::new("email.send", 1)));
        let result = runtime.invoke_capability("email.send", &[10, 20]).unwrap();
        assert_eq!(result.outputs.len(), 1);
    }

    #[test]
    fn runtime_errors_on_missing_provider() {
        let runtime = Runtime::sandbox();
        let err = runtime.invoke_capability("nonexistent", &[]).unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
    }

    #[test]
    fn validate_module_catches_missing_providers() {
        let module = make_test_module();
        let runtime = Runtime::sandbox();
        let missing = runtime.validate_module(&module).unwrap_err();
        assert_eq!(missing, vec!["email.send"]);
    }

    #[test]
    fn register_fakes_covers_all_capabilities() {
        let module = make_test_module();
        let mut runtime = Runtime::sandbox();
        runtime.register_fakes_for_module(&module);
        assert!(runtime.has_provider("email.send"));
        assert!(runtime.validate_module(&module).is_ok());
    }

    #[test]
    fn approval_policy_always_deny() {
        let runtime = Runtime::sandbox().with_approval_policy(ApprovalPolicy::AlwaysDeny);
        assert!(!runtime.check_approval());
    }

    #[test]
    fn approval_policy_always_allow() {
        let runtime = Runtime::sandbox();
        assert!(runtime.check_approval());
    }
}
