use serde::{Deserialize, Serialize};
use std::fmt;

/// A single trace event recorded during interpretation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub id: usize,
    pub kind: TraceEventKind,
    pub function: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TraceEventKind {
    FunctionEntry,
    FunctionReturn {
        value: Option<String>,
    },
    MemoryAlloc {
        region: usize,
        size: usize,
    },
    MemoryLoad {
        region: usize,
        offset: usize,
    },
    MemoryStore {
        region: usize,
        offset: usize,
        value: i64,
    },
    ProofChecked {
        description: String,
    },
    BranchTaken {
        target: String,
    },
    AssertPassed,
    AssertFailed,
    Call {
        callee: String,
    },
    ApprovalRequested,
    CapabilityInvoked {
        capability: String,
    },
    TaskSpawned {
        task: String,
    },
    TaskAwaited {
        task: String,
    },
    Checkpoint {
        label: String,
    },
}

impl TraceEventKind {
    /// Returns true if this is an externally observable event (visible to callers/users).
    pub fn is_observable(&self) -> bool {
        matches!(
            self,
            TraceEventKind::FunctionEntry
                | TraceEventKind::FunctionReturn { .. }
                | TraceEventKind::CapabilityInvoked { .. }
                | TraceEventKind::ApprovalRequested
                | TraceEventKind::Call { .. }
        )
    }
}

impl fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "arc.event @e{} ", self.id)?;
        match &self.kind {
            TraceEventKind::FunctionEntry => {
                write!(f, "invoke @{}", self.function)
            }
            TraceEventKind::FunctionReturn { value } => match value {
                Some(v) => write!(f, "returned @{} value={}", self.function, v),
                None => write!(f, "returned @{}", self.function),
            },
            TraceEventKind::MemoryAlloc { region, size } => {
                write!(f, "memory.alloc region={} size={}", region, size)
            }
            TraceEventKind::MemoryLoad { region, offset } => {
                write!(f, "memory.load region={} offset={}", region, offset)
            }
            TraceEventKind::MemoryStore {
                region,
                offset,
                value,
            } => {
                write!(
                    f,
                    "memory.store region={} offset={} value={}",
                    region, offset, value
                )
            }
            TraceEventKind::ProofChecked { description } => {
                write!(f, "proof.checked {}", description)
            }
            TraceEventKind::BranchTaken { target } => {
                write!(f, "branch.taken ^{}", target)
            }
            TraceEventKind::AssertPassed => write!(f, "assert.passed"),
            TraceEventKind::AssertFailed => write!(f, "assert.failed"),
            TraceEventKind::Call { callee } => {
                write!(f, "call @{}", callee)
            }
            TraceEventKind::ApprovalRequested => write!(f, "approval.requested"),
            TraceEventKind::CapabilityInvoked { capability } => {
                write!(f, "capability.invoked @{}", capability)
            }
            TraceEventKind::TaskSpawned { task } => {
                write!(f, "task.spawned @{}", task)
            }
            TraceEventKind::TaskAwaited { task } => {
                write!(f, "task.awaited @{}", task)
            }
            TraceEventKind::Checkpoint { label } => {
                write!(f, "checkpoint @{}", label)
            }
        }
    }
}

/// Result of comparing two traces.
#[derive(Debug, Clone)]
pub struct TraceComparison {
    pub matches: bool,
    pub event_count_match: bool,
    pub first_divergence: Option<usize>,
    pub summary: String,
}

/// Execution trace collecting all events from an interpreted run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Trace {
    events: Vec<TraceEvent>,
    next_id: usize,
}

impl Trace {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            next_id: 0,
        }
    }

    pub fn record(&mut self, function: &str, kind: TraceEventKind) {
        let id = self.next_id;
        self.next_id += 1;
        self.events.push(TraceEvent {
            id,
            kind,
            function: function.to_string(),
        });
    }

    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Format the trace as an AIR trace block.
    pub fn format(&self, trace_name: &str) -> String {
        let mut out = format!("arc.trace @{} {{\n", trace_name);
        for event in &self.events {
            out.push_str(&format!("  {}\n", event));
        }
        out.push_str("}\n");
        out
    }

    /// Serialize the trace to JSON for storage/transport.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("trace serialization cannot fail")
    }

    /// Deserialize a trace from JSON.
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("trace deserialization error: {}", e))
    }

    /// Return only externally observable events (function entry/return, capabilities, approvals).
    pub fn observable_events(&self) -> Vec<&TraceEvent> {
        self.events
            .iter()
            .filter(|e| e.kind.is_observable())
            .collect()
    }

    /// Filter events whose Display output contains the given substring.
    pub fn filter_events(&self, pattern: &str) -> Vec<&TraceEvent> {
        self.events
            .iter()
            .filter(|e| e.to_string().contains(pattern))
            .collect()
    }

    /// Compare this trace against another, checking observable event equivalence.
    ///
    /// Two traces match if their observable events (function entry/return,
    /// capability invocations, approvals) occur in the same order with the
    /// same parameters. Internal events (branches, memory, proofs) may differ.
    pub fn compare(&self, other: &Trace) -> TraceComparison {
        let self_obs = self.observable_events();
        let other_obs = other.observable_events();

        let event_count_match = self_obs.len() == other_obs.len();
        let mut first_divergence = None;

        let check_len = self_obs.len().min(other_obs.len());
        for i in 0..check_len {
            if self_obs[i].kind != other_obs[i].kind
                || self_obs[i].function != other_obs[i].function
            {
                first_divergence = Some(i);
                break;
            }
        }

        if first_divergence.is_none() && !event_count_match {
            first_divergence = Some(check_len);
        }

        let matches = event_count_match && first_divergence.is_none();

        let summary = if matches {
            format!("traces match ({} observable events)", self_obs.len())
        } else if let Some(idx) = first_divergence {
            let self_desc = self_obs
                .get(idx)
                .map(|e| e.to_string())
                .unwrap_or_else(|| "<end>".into());
            let other_desc = other_obs
                .get(idx)
                .map(|e| e.to_string())
                .unwrap_or_else(|| "<end>".into());
            format!(
                "divergence at observable event {}: left=[{}] right=[{}]",
                idx, self_desc, other_desc
            )
        } else {
            "traces differ in event count".into()
        };

        TraceComparison {
            matches,
            event_count_match,
            first_divergence,
            summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_records_events() {
        let mut trace = Trace::new();
        trace.record("main", TraceEventKind::FunctionEntry);
        trace.record("main", TraceEventKind::MemoryAlloc { region: 0, size: 4 });
        trace.record(
            "main",
            TraceEventKind::FunctionReturn {
                value: Some("42".to_string()),
            },
        );
        assert_eq!(trace.len(), 3);
        assert_eq!(trace.events()[0].id, 0);
        assert_eq!(trace.events()[2].id, 2);
    }

    #[test]
    fn trace_format_output() {
        let mut trace = Trace::new();
        trace.record("main", TraceEventKind::FunctionEntry);
        trace.record("main", TraceEventKind::AssertPassed);
        let formatted = trace.format("run_001");
        assert!(formatted.contains("arc.trace @run_001"));
        assert!(formatted.contains("invoke @main"));
        assert!(formatted.contains("assert.passed"));
    }

    #[test]
    fn trace_json_roundtrip() {
        let mut trace = Trace::new();
        trace.record("main", TraceEventKind::FunctionEntry);
        trace.record(
            "main",
            TraceEventKind::BranchTaken {
                target: "loop".into(),
            },
        );
        trace.record(
            "main",
            TraceEventKind::CapabilityInvoked {
                capability: "email.send".into(),
            },
        );
        trace.record(
            "main",
            TraceEventKind::FunctionReturn {
                value: Some("0".into()),
            },
        );

        let json = trace.to_json();
        let restored = Trace::from_json(&json).unwrap();
        assert_eq!(restored.len(), trace.len());
        assert_eq!(restored.events()[2].kind, trace.events()[2].kind);
    }

    #[test]
    fn trace_observable_events() {
        let mut trace = Trace::new();
        trace.record("main", TraceEventKind::FunctionEntry);
        trace.record(
            "main",
            TraceEventKind::BranchTaken {
                target: "bb1".into(),
            },
        );
        trace.record("main", TraceEventKind::AssertPassed);
        trace.record(
            "main",
            TraceEventKind::CapabilityInvoked {
                capability: "fs.read".into(),
            },
        );
        trace.record("main", TraceEventKind::FunctionReturn { value: None });

        let obs = trace.observable_events();
        assert_eq!(obs.len(), 3); // entry, capability, return
    }

    #[test]
    fn trace_filter_events() {
        let mut trace = Trace::new();
        trace.record("main", TraceEventKind::FunctionEntry);
        trace.record(
            "main",
            TraceEventKind::CapabilityInvoked {
                capability: "email.send".into(),
            },
        );
        trace.record(
            "main",
            TraceEventKind::CapabilityInvoked {
                capability: "file.read".into(),
            },
        );

        let filtered = trace.filter_events("email");
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].to_string().contains("email.send"));
    }

    #[test]
    fn trace_compare_identical() {
        let mut t1 = Trace::new();
        t1.record("main", TraceEventKind::FunctionEntry);
        t1.record(
            "main",
            TraceEventKind::CapabilityInvoked {
                capability: "x".into(),
            },
        );
        t1.record(
            "main",
            TraceEventKind::FunctionReturn {
                value: Some("1".into()),
            },
        );

        let mut t2 = Trace::new();
        t2.record("main", TraceEventKind::FunctionEntry);
        t2.record(
            "main",
            TraceEventKind::CapabilityInvoked {
                capability: "x".into(),
            },
        );
        t2.record(
            "main",
            TraceEventKind::FunctionReturn {
                value: Some("1".into()),
            },
        );

        let cmp = t1.compare(&t2);
        assert!(cmp.matches);
        assert!(cmp.event_count_match);
        assert!(cmp.first_divergence.is_none());
    }

    #[test]
    fn trace_compare_different_internal_events_still_match() {
        let mut t1 = Trace::new();
        t1.record("main", TraceEventKind::FunctionEntry);
        t1.record(
            "main",
            TraceEventKind::BranchTaken {
                target: "bb1".into(),
            },
        );
        t1.record("main", TraceEventKind::FunctionReturn { value: None });

        let mut t2 = Trace::new();
        t2.record("main", TraceEventKind::FunctionEntry);
        t2.record(
            "main",
            TraceEventKind::BranchTaken {
                target: "bb2".into(),
            },
        );
        t2.record("main", TraceEventKind::AssertPassed);
        t2.record("main", TraceEventKind::FunctionReturn { value: None });

        let cmp = t1.compare(&t2);
        assert!(
            cmp.matches,
            "internal events should be ignored: {}",
            cmp.summary
        );
    }

    #[test]
    fn trace_compare_divergent() {
        let mut t1 = Trace::new();
        t1.record("main", TraceEventKind::FunctionEntry);
        t1.record(
            "main",
            TraceEventKind::CapabilityInvoked {
                capability: "a".into(),
            },
        );
        t1.record("main", TraceEventKind::FunctionReturn { value: None });

        let mut t2 = Trace::new();
        t2.record("main", TraceEventKind::FunctionEntry);
        t2.record(
            "main",
            TraceEventKind::CapabilityInvoked {
                capability: "b".into(),
            },
        );
        t2.record("main", TraceEventKind::FunctionReturn { value: None });

        let cmp = t1.compare(&t2);
        assert!(!cmp.matches);
        assert_eq!(cmp.first_divergence, Some(1));
    }

    #[test]
    fn trace_compare_different_length() {
        let mut t1 = Trace::new();
        t1.record("main", TraceEventKind::FunctionEntry);

        let mut t2 = Trace::new();
        t2.record("main", TraceEventKind::FunctionEntry);
        t2.record("main", TraceEventKind::FunctionReturn { value: None });

        let cmp = t1.compare(&t2);
        assert!(!cmp.matches);
        assert!(!cmp.event_count_match);
    }
}
