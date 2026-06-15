//! AIR concurrency: async/await, structured concurrency, continuations,
//! checkpoint/resume, cancellation, and timeouts.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Task / Future identifiers
// ---------------------------------------------------------------------------

/// A unique identifier for an async task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "task:{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// Task state machine
// ---------------------------------------------------------------------------

/// The lifecycle state of an async task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState {
    /// Created but not yet started.
    Pending,
    /// Actively executing.
    Running,
    /// Waiting for another task or external input.
    Suspended { reason: SuspendReason },
    /// Successfully completed.
    Completed { result: TaskResult },
    /// Failed with an error.
    Failed { error: String },
    /// Cancelled before completion.
    Cancelled,
}

/// Why a task was suspended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuspendReason {
    /// Waiting for user approval.
    WaitingForApproval { resource: String },
    /// Waiting for another task to complete.
    WaitingForTask(TaskId),
    /// Waiting for external input.
    WaitingForInput { input_type: String },
    /// Waiting for a timeout.
    Timeout { duration_ms: u64 },
    /// Explicit checkpoint (can be serialized and resumed later).
    Checkpoint { label: String },
}

/// The result of a completed task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskResult {
    Value(i64),
    Text(String),
    Void,
}

// ---------------------------------------------------------------------------
// Continuation
// ---------------------------------------------------------------------------

/// A serializable continuation representing a suspended computation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Continuation {
    pub task_id: TaskId,
    pub label: String,
    /// Captured variable bindings at the suspend point.
    pub captures: HashMap<String, CapturedValue>,
    /// What the continuation expects to receive when resumed.
    pub resume_type: String,
    /// The program counter / instruction index to resume at.
    pub resume_point: usize,
}

/// A captured value in a continuation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapturedValue {
    Int(i64),
    Text(String),
    Bool(bool),
    Ref(String), // reference to another value by name
}

impl Continuation {
    pub fn new(
        task_id: TaskId,
        label: impl Into<String>,
        resume_type: impl Into<String>,
        resume_point: usize,
    ) -> Self {
        Self {
            task_id,
            label: label.into(),
            captures: HashMap::new(),
            resume_type: resume_type.into(),
            resume_point,
        }
    }

    pub fn capture(&mut self, name: impl Into<String>, value: CapturedValue) {
        self.captures.insert(name.into(), value);
    }

    /// Serialize to JSON for checkpointing.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

// ---------------------------------------------------------------------------
// Structured concurrency
// ---------------------------------------------------------------------------

/// A structured concurrency scope: all child tasks must complete before
/// the scope exits.
#[derive(Debug)]
pub struct ConcurrencyScope {
    pub id: String,
    pub tasks: Vec<TaskId>,
    pub states: HashMap<TaskId, TaskState>,
    pub timeout: Option<Duration>,
}

impl ConcurrencyScope {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            tasks: Vec::new(),
            states: HashMap::new(),
            timeout: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Spawn a new task in this scope.
    pub fn spawn(&mut self, task_id: TaskId) {
        self.states.insert(task_id.clone(), TaskState::Pending);
        self.tasks.push(task_id);
    }

    /// Update the state of a task.
    pub fn update(&mut self, task_id: &TaskId, state: TaskState) -> Result<(), AsyncError> {
        if !self.states.contains_key(task_id) {
            return Err(AsyncError::UnknownTask(task_id.clone()));
        }
        self.states.insert(task_id.clone(), state);
        Ok(())
    }

    /// Check if all tasks in the scope have completed (or failed/cancelled).
    pub fn is_complete(&self) -> bool {
        self.states.values().all(|s| {
            matches!(
                s,
                TaskState::Completed { .. } | TaskState::Failed { .. } | TaskState::Cancelled
            )
        })
    }

    /// Check if any task has failed.
    pub fn has_failures(&self) -> bool {
        self.states
            .values()
            .any(|s| matches!(s, TaskState::Failed { .. }))
    }

    /// Get all completed results.
    pub fn results(&self) -> Vec<(&TaskId, &TaskResult)> {
        self.states
            .iter()
            .filter_map(|(id, state)| {
                if let TaskState::Completed { result } = state {
                    Some((id, result))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Cancel all pending/running tasks.
    pub fn cancel_all(&mut self) {
        for state in self.states.values_mut() {
            if matches!(
                state,
                TaskState::Pending | TaskState::Running | TaskState::Suspended { .. }
            ) {
                *state = TaskState::Cancelled;
            }
        }
    }

    /// Get tasks that are ready to run (pending).
    pub fn pending_tasks(&self) -> Vec<&TaskId> {
        self.states
            .iter()
            .filter(|(_, s)| matches!(s, TaskState::Pending))
            .map(|(id, _)| id)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Cancellation token
// ---------------------------------------------------------------------------

/// A token that can signal cancellation to async tasks.
#[derive(Debug, Clone)]
pub struct CancellationToken {
    cancelled: bool,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self { cancelled: false }
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Checkpoint store
// ---------------------------------------------------------------------------

/// A store for checkpointed continuations, enabling durable execution.
#[derive(Debug, Default)]
pub struct CheckpointStore {
    continuations: HashMap<String, String>, // label -> JSON
}

impl CheckpointStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Save a continuation checkpoint.
    pub fn save(&mut self, continuation: &Continuation) -> Result<(), AsyncError> {
        let json = continuation
            .to_json()
            .map_err(|e| AsyncError::SerializationError(e.to_string()))?;
        self.continuations.insert(continuation.label.clone(), json);
        Ok(())
    }

    /// Load a continuation from checkpoint.
    pub fn load(&self, label: &str) -> Result<Continuation, AsyncError> {
        let json = self
            .continuations
            .get(label)
            .ok_or_else(|| AsyncError::CheckpointNotFound(label.to_string()))?;
        Continuation::from_json(json).map_err(|e| AsyncError::SerializationError(e.to_string()))
    }

    /// Remove a checkpoint.
    pub fn remove(&mut self, label: &str) -> bool {
        self.continuations.remove(label).is_some()
    }

    /// List all checkpoint labels.
    pub fn labels(&self) -> Vec<&str> {
        self.continuations.keys().map(|s| s.as_str()).collect()
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AsyncError {
    #[error("unknown task: {0}")]
    UnknownTask(TaskId),
    #[error("task already completed: {0}")]
    AlreadyCompleted(TaskId),
    #[error("checkpoint not found: {0}")]
    CheckpointNotFound(String),
    #[error("serialization error: {0}")]
    SerializationError(String),
    #[error("timeout exceeded")]
    Timeout,
    #[error("cancelled")]
    Cancelled,
    #[error("non-commuting effects in parallel: {0}")]
    NonCommutingParallel(String),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_lifecycle() {
        let mut scope = ConcurrencyScope::new("scope1");
        let t1 = TaskId::new("t1");
        let t2 = TaskId::new("t2");

        scope.spawn(t1.clone());
        scope.spawn(t2.clone());

        assert!(!scope.is_complete());
        assert_eq!(scope.pending_tasks().len(), 2);

        scope.update(&t1, TaskState::Running).unwrap();
        scope
            .update(
                &t1,
                TaskState::Completed {
                    result: TaskResult::Value(42),
                },
            )
            .unwrap();
        scope
            .update(
                &t2,
                TaskState::Completed {
                    result: TaskResult::Value(99),
                },
            )
            .unwrap();

        assert!(scope.is_complete());
        assert!(!scope.has_failures());
        assert_eq!(scope.results().len(), 2);
    }

    #[test]
    fn task_failure() {
        let mut scope = ConcurrencyScope::new("scope2");
        let t1 = TaskId::new("t1");
        scope.spawn(t1.clone());
        scope
            .update(
                &t1,
                TaskState::Failed {
                    error: "network timeout".into(),
                },
            )
            .unwrap();

        assert!(scope.is_complete());
        assert!(scope.has_failures());
    }

    #[test]
    fn cancel_all() {
        let mut scope = ConcurrencyScope::new("scope3");
        scope.spawn(TaskId::new("a"));
        scope.spawn(TaskId::new("b"));
        scope.spawn(TaskId::new("c"));

        scope.update(&TaskId::new("a"), TaskState::Running).unwrap();
        scope
            .update(
                &TaskId::new("b"),
                TaskState::Completed {
                    result: TaskResult::Void,
                },
            )
            .unwrap();

        scope.cancel_all();

        // "a" was running -> cancelled, "c" was pending -> cancelled.
        // "b" was completed -> stays completed.
        assert!(scope.is_complete());
        let cancelled_count = scope
            .states
            .values()
            .filter(|s| matches!(s, TaskState::Cancelled))
            .count();
        assert_eq!(cancelled_count, 2);
    }

    #[test]
    fn unknown_task_error() {
        let mut scope = ConcurrencyScope::new("scope4");
        let result = scope.update(&TaskId::new("nonexistent"), TaskState::Running);
        assert!(result.is_err());
    }

    #[test]
    fn continuation_roundtrip() {
        let mut cont = Continuation::new(
            TaskId::new("task_1"),
            "checkpoint_1",
            "!arc.auth<email.send>",
            42,
        );
        cont.capture("draft", CapturedValue::Text("Hello".into()));
        cont.capture("counter", CapturedValue::Int(7));

        let json = cont.to_json().unwrap();
        let restored = Continuation::from_json(&json).unwrap();

        assert_eq!(restored.task_id, TaskId::new("task_1"));
        assert_eq!(restored.label, "checkpoint_1");
        assert_eq!(restored.resume_point, 42);
        assert_eq!(
            restored.captures.get("draft"),
            Some(&CapturedValue::Text("Hello".into()))
        );
        assert_eq!(
            restored.captures.get("counter"),
            Some(&CapturedValue::Int(7))
        );
    }

    #[test]
    fn checkpoint_store_save_load() {
        let mut store = CheckpointStore::new();
        let cont = Continuation::new(TaskId::new("t1"), "cp_email", "!arc.auth<send>", 10);
        store.save(&cont).unwrap();

        let labels = store.labels();
        assert_eq!(labels.len(), 1);
        assert!(labels.contains(&"cp_email"));

        let loaded = store.load("cp_email").unwrap();
        assert_eq!(loaded.task_id, TaskId::new("t1"));
    }

    #[test]
    fn checkpoint_store_remove() {
        let mut store = CheckpointStore::new();
        let cont = Continuation::new(TaskId::new("t1"), "cp1", "i64", 0);
        store.save(&cont).unwrap();
        assert!(store.remove("cp1"));
        assert!(!store.remove("cp1")); // already removed
        assert!(store.load("cp1").is_err());
    }

    #[test]
    fn cancellation_token() {
        let mut token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn suspend_reasons() {
        let suspended = TaskState::Suspended {
            reason: SuspendReason::WaitingForApproval {
                resource: "email.send".into(),
            },
        };
        assert!(matches!(
            suspended,
            TaskState::Suspended {
                reason: SuspendReason::WaitingForApproval { .. }
            }
        ));

        let checkpoint = TaskState::Suspended {
            reason: SuspendReason::Checkpoint {
                label: "cp1".into(),
            },
        };
        assert!(matches!(
            checkpoint,
            TaskState::Suspended {
                reason: SuspendReason::Checkpoint { .. }
            }
        ));
    }

    #[test]
    fn scope_with_timeout() {
        let scope = ConcurrencyScope::new("timed").with_timeout(Duration::from_secs(30));
        assert_eq!(scope.timeout, Some(Duration::from_secs(30)));
    }

    #[test]
    fn task_result_variants() {
        let v = TaskResult::Value(42);
        let t = TaskResult::Text("hello".into());
        let n = TaskResult::Void;

        assert_eq!(v, TaskResult::Value(42));
        assert_eq!(t, TaskResult::Text("hello".into()));
        assert_eq!(n, TaskResult::Void);
    }
}
