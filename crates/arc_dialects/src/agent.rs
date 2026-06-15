//! Agent dialect: typed agent workflows, tasks, checkpoints, and orchestration.

use std::collections::HashMap;
use std::fmt;

/// A capability requirement for an agent task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityReq {
    pub name: String,
    pub effects: Vec<String>,
    pub requires_approval: bool,
}

impl CapabilityReq {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            effects: Vec::new(),
            requires_approval: false,
        }
    }

    pub fn with_effect(mut self, effect: impl Into<String>) -> Self {
        self.effects.push(effect.into());
        self
    }

    pub fn with_approval(mut self) -> Self {
        self.requires_approval = true;
        self
    }
}

/// A step in an agent workflow.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskStep {
    /// Invoke a capability.
    Invoke {
        capability: String,
        args: Vec<String>,
        result: Option<String>,
    },
    /// Request human approval.
    Approve {
        principal: String,
        resource: String,
        result: String,
    },
    /// Conditional branch.
    Condition {
        predicate: String,
        then_steps: Vec<TaskStep>,
        else_steps: Vec<TaskStep>,
    },
    /// Execute steps in parallel (only if effects commute).
    Parallel { branches: Vec<Vec<TaskStep>> },
    /// Checkpoint: save state for potential resume.
    Checkpoint {
        label: String,
        captures: Vec<String>,
    },
    /// Suspend: wait for external input.
    Suspend { reason: String, resume_type: String },
    /// Log/trace an event.
    Trace { event: String, data: Vec<String> },
}

/// An agent task definition.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskDef {
    pub name: String,
    pub inputs: Vec<(String, String)>,  // (name, type)
    pub outputs: Vec<(String, String)>, // (name, type)
    pub capabilities: Vec<CapabilityReq>,
    pub steps: Vec<TaskStep>,
    pub rollback: Vec<TaskStep>,
}

impl TaskDef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            capabilities: Vec::new(),
            steps: Vec::new(),
            rollback: Vec::new(),
        }
    }

    pub fn add_input(&mut self, name: impl Into<String>, ty: impl Into<String>) {
        self.inputs.push((name.into(), ty.into()));
    }

    pub fn add_output(&mut self, name: impl Into<String>, ty: impl Into<String>) {
        self.outputs.push((name.into(), ty.into()));
    }

    pub fn add_capability(&mut self, cap: CapabilityReq) {
        self.capabilities.push(cap);
    }

    pub fn add_step(&mut self, step: TaskStep) {
        self.steps.push(step);
    }

    pub fn add_rollback_step(&mut self, step: TaskStep) {
        self.rollback.push(step);
    }
}

/// Validation errors for agent tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskError {
    /// A capability is used but not declared.
    UndeclaredCapability(String),
    /// Approval required but not obtained before capability use.
    MissingApproval { capability: String },
    /// Parallel steps have non-commuting effects.
    NonCommutingParallel { step_a: String, step_b: String },
    /// Checkpoint references undefined variable.
    UndefinedCapture(String),
    /// Rollback not available for irreversible step.
    IrreversibleWithoutRollback(String),
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UndeclaredCapability(c) => write!(f, "undeclared capability: {}", c),
            Self::MissingApproval { capability } => {
                write!(f, "missing approval for: {}", capability)
            }
            Self::NonCommutingParallel { step_a, step_b } => {
                write!(f, "non-commuting parallel: {} and {}", step_a, step_b)
            }
            Self::UndefinedCapture(v) => write!(f, "undefined capture: {}", v),
            Self::IrreversibleWithoutRollback(s) => {
                write!(f, "irreversible without rollback: {}", s)
            }
        }
    }
}

/// Validate an agent task definition.
pub fn validate_task(task: &TaskDef) -> Vec<TaskError> {
    let mut errors = Vec::new();
    let declared: HashMap<&str, &CapabilityReq> = task
        .capabilities
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();
    let mut approved: Vec<String> = Vec::new();
    let mut defined: Vec<String> = task.inputs.iter().map(|(n, _)| n.clone()).collect();

    validate_steps(
        &task.steps,
        &declared,
        &mut approved,
        &mut defined,
        &mut errors,
    );

    errors
}

fn validate_steps(
    steps: &[TaskStep],
    declared: &HashMap<&str, &CapabilityReq>,
    approved: &mut Vec<String>,
    defined: &mut Vec<String>,
    errors: &mut Vec<TaskError>,
) {
    for step in steps {
        match step {
            TaskStep::Invoke {
                capability, result, ..
            } => {
                if !declared.contains_key(capability.as_str()) {
                    errors.push(TaskError::UndeclaredCapability(capability.clone()));
                } else {
                    let cap = declared[capability.as_str()];
                    if cap.requires_approval && !approved.contains(capability) {
                        errors.push(TaskError::MissingApproval {
                            capability: capability.clone(),
                        });
                    }
                }
                if let Some(r) = result {
                    defined.push(r.clone());
                }
            }
            TaskStep::Approve {
                resource, result, ..
            } => {
                // Record that this resource/capability has been approved.
                approved.push(resource.clone());
                defined.push(result.clone());
            }
            TaskStep::Condition {
                then_steps,
                else_steps,
                ..
            } => {
                let mut then_approved = approved.clone();
                let mut then_defined = defined.clone();
                validate_steps(
                    then_steps,
                    declared,
                    &mut then_approved,
                    &mut then_defined,
                    errors,
                );

                let mut else_approved = approved.clone();
                let mut else_defined = defined.clone();
                validate_steps(
                    else_steps,
                    declared,
                    &mut else_approved,
                    &mut else_defined,
                    errors,
                );
            }
            TaskStep::Parallel { branches } => {
                for branch in branches {
                    let mut branch_approved = approved.clone();
                    let mut branch_defined = defined.clone();
                    validate_steps(
                        branch,
                        declared,
                        &mut branch_approved,
                        &mut branch_defined,
                        errors,
                    );
                }
            }
            TaskStep::Checkpoint { captures, .. } => {
                for cap in captures {
                    if !defined.contains(cap) {
                        errors.push(TaskError::UndefinedCapture(cap.clone()));
                    }
                }
            }
            TaskStep::Suspend { .. } | TaskStep::Trace { .. } => {}
        }
    }
}

/// Compute the full set of effects for a task.
pub fn task_effects(task: &TaskDef) -> Vec<String> {
    let mut effects: Vec<String> = Vec::new();
    for cap in &task.capabilities {
        for eff in &cap.effects {
            if !effects.contains(eff) {
                effects.push(eff.clone());
            }
        }
    }
    effects
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email_task() -> TaskDef {
        let mut task = TaskDef::new("send_report");
        task.add_input("doc", "!arc.document");
        task.add_input("user", "!arc.principal");
        task.add_output("world", "!arc.world");

        task.add_capability(CapabilityReq::new("model.summarize").with_effect("llm"));
        task.add_capability(
            CapabilityReq::new("email.send")
                .with_effect("external_communication")
                .with_effect("irreversible")
                .with_approval(),
        );

        task.add_step(TaskStep::Invoke {
            capability: "model.summarize".into(),
            args: vec!["doc".into()],
            result: Some("summary".into()),
        });
        task.add_step(TaskStep::Approve {
            principal: "user".into(),
            resource: "email.send".into(),
            result: "auth".into(),
        });
        task.add_step(TaskStep::Invoke {
            capability: "email.send".into(),
            args: vec!["summary".into()],
            result: Some("msg".into()),
        });

        task
    }

    #[test]
    fn valid_task_passes() {
        let task = email_task();
        let errors = validate_task(&task);
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn undeclared_capability_detected() {
        let mut task = TaskDef::new("bad");
        task.add_step(TaskStep::Invoke {
            capability: "unknown.tool".into(),
            args: vec![],
            result: None,
        });
        let errors = validate_task(&task);
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], TaskError::UndeclaredCapability(c) if c == "unknown.tool"));
    }

    #[test]
    fn missing_approval_detected() {
        let mut task = TaskDef::new("no_approval");
        task.add_capability(CapabilityReq::new("email.send").with_approval());
        // Invoke without getting approval first.
        task.add_step(TaskStep::Invoke {
            capability: "email.send".into(),
            args: vec![],
            result: None,
        });
        let errors = validate_task(&task);
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], TaskError::MissingApproval { .. }));
    }

    #[test]
    fn approval_before_invoke_passes() {
        let mut task = TaskDef::new("with_approval");
        task.add_capability(CapabilityReq::new("email.send").with_approval());
        task.add_input("user", "!arc.principal");
        task.add_step(TaskStep::Approve {
            principal: "user".into(),
            resource: "email.send".into(),
            result: "auth".into(),
        });
        task.add_step(TaskStep::Invoke {
            capability: "email.send".into(),
            args: vec![],
            result: None,
        });
        let errors = validate_task(&task);
        assert!(errors.is_empty());
    }

    #[test]
    fn undefined_capture_detected() {
        let mut task = TaskDef::new("bad_checkpoint");
        task.add_step(TaskStep::Checkpoint {
            label: "cp1".into(),
            captures: vec!["nonexistent".into()],
        });
        let errors = validate_task(&task);
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], TaskError::UndefinedCapture(v) if v == "nonexistent"));
    }

    #[test]
    fn task_effects_computed() {
        let task = email_task();
        let effects = task_effects(&task);
        assert!(effects.contains(&"llm".to_string()));
        assert!(effects.contains(&"external_communication".to_string()));
        assert!(effects.contains(&"irreversible".to_string()));
    }

    #[test]
    fn conditional_task() {
        let mut task = TaskDef::new("conditional");
        task.add_input("flag", "i1");
        task.add_capability(CapabilityReq::new("tool.a"));
        task.add_capability(CapabilityReq::new("tool.b"));

        task.add_step(TaskStep::Condition {
            predicate: "flag".into(),
            then_steps: vec![TaskStep::Invoke {
                capability: "tool.a".into(),
                args: vec![],
                result: None,
            }],
            else_steps: vec![TaskStep::Invoke {
                capability: "tool.b".into(),
                args: vec![],
                result: None,
            }],
        });
        let errors = validate_task(&task);
        assert!(errors.is_empty());
    }

    #[test]
    fn rollback_steps() {
        let mut task = TaskDef::new("with_rollback");
        task.add_capability(CapabilityReq::new("file.copy").with_effect("filesystem.write"));

        task.add_step(TaskStep::Invoke {
            capability: "file.copy".into(),
            args: vec![],
            result: None,
        });
        task.add_rollback_step(TaskStep::Trace {
            event: "rollback.file.copy".into(),
            data: vec![],
        });

        assert!(!task.rollback.is_empty());
    }
}
