use std::fmt;

/// An individual effect category in the AIR effect lattice.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Effect {
    Pure,
    MemoryRead,
    MemoryWrite,
    Allocate,
    Deallocate,
    FilesystemRead,
    FilesystemWrite,
    Network,
    DatabaseRead,
    DatabaseWrite,
    Ui,
    Llm,
    HumanApproval,
    ExternalCommunication,
    ExternalMutation,
    Financial,
    Credential,
    Physical,
    Irreversible,
}

impl Effect {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pure" => Some(Self::Pure),
            "memory.read" => Some(Self::MemoryRead),
            "memory.write" => Some(Self::MemoryWrite),
            "allocate" => Some(Self::Allocate),
            "deallocate" => Some(Self::Deallocate),
            "filesystem.read" => Some(Self::FilesystemRead),
            "filesystem.write" => Some(Self::FilesystemWrite),
            "network" => Some(Self::Network),
            "database.read" => Some(Self::DatabaseRead),
            "database.write" => Some(Self::DatabaseWrite),
            "ui" => Some(Self::Ui),
            "llm" => Some(Self::Llm),
            "human.approval" => Some(Self::HumanApproval),
            "external_communication" => Some(Self::ExternalCommunication),
            "external_mutation" => Some(Self::ExternalMutation),
            "financial" => Some(Self::Financial),
            "credential" => Some(Self::Credential),
            "physical" => Some(Self::Physical),
            "irreversible" => Some(Self::Irreversible),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::MemoryRead => "memory.read",
            Self::MemoryWrite => "memory.write",
            Self::Allocate => "allocate",
            Self::Deallocate => "deallocate",
            Self::FilesystemRead => "filesystem.read",
            Self::FilesystemWrite => "filesystem.write",
            Self::Network => "network",
            Self::DatabaseRead => "database.read",
            Self::DatabaseWrite => "database.write",
            Self::Ui => "ui",
            Self::Llm => "llm",
            Self::HumanApproval => "human.approval",
            Self::ExternalCommunication => "external_communication",
            Self::ExternalMutation => "external_mutation",
            Self::Financial => "financial",
            Self::Credential => "credential",
            Self::Physical => "physical",
            Self::Irreversible => "irreversible",
        }
    }

    /// Returns true if this effect is side-effect-free (can be reordered/eliminated).
    pub fn is_pure(&self) -> bool {
        matches!(self, Self::Pure)
    }

    /// Returns true if this effect is external (crosses the program boundary).
    pub fn is_external(&self) -> bool {
        matches!(
            self,
            Self::Network
                | Self::ExternalCommunication
                | Self::ExternalMutation
                | Self::Financial
                | Self::Physical
                | Self::Irreversible
        )
    }

    /// Returns true if this effect is a read-only observation.
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::Pure | Self::MemoryRead | Self::FilesystemRead | Self::DatabaseRead
        )
    }

    /// Returns true if two effects commute (can be safely reordered).
    pub fn commutes_with(&self, other: &Effect) -> bool {
        if self.is_pure() || other.is_pure() {
            return true;
        }
        if self.is_read_only() && other.is_read_only() {
            return true;
        }
        false
    }
}

impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Effect qualifier describing properties of an effectful operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EffectQualifier {
    Deterministic,
    Nondeterministic,
    Idempotent,
    Reversible,
    Transactional,
    Blocking,
    Async,
    MayFail,
    MayTimeout,
    Speculatable,
    Commutative,
}

impl EffectQualifier {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "deterministic" => Some(Self::Deterministic),
            "nondeterministic" => Some(Self::Nondeterministic),
            "idempotent" => Some(Self::Idempotent),
            "reversible" => Some(Self::Reversible),
            "transactional" => Some(Self::Transactional),
            "blocking" => Some(Self::Blocking),
            "async" => Some(Self::Async),
            "may_fail" => Some(Self::MayFail),
            "may_timeout" => Some(Self::MayTimeout),
            "speculatable" => Some(Self::Speculatable),
            "commutative" => Some(Self::Commutative),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::Nondeterministic => "nondeterministic",
            Self::Idempotent => "idempotent",
            Self::Reversible => "reversible",
            Self::Transactional => "transactional",
            Self::Blocking => "blocking",
            Self::Async => "async",
            Self::MayFail => "may_fail",
            Self::MayTimeout => "may_timeout",
            Self::Speculatable => "speculatable",
            Self::Commutative => "commutative",
        }
    }
}

impl fmt::Display for EffectQualifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A set of effects declared on an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectSet {
    effects: Vec<Effect>,
}

impl EffectSet {
    pub fn new(effects: Vec<Effect>) -> Self {
        Self { effects }
    }

    pub fn pure() -> Self {
        Self {
            effects: Vec::new(),
        }
    }

    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    pub fn is_pure(&self) -> bool {
        self.effects.is_empty() || self.effects.iter().all(|e| e.is_pure())
    }

    pub fn has_external_effects(&self) -> bool {
        self.effects.iter().any(|e| e.is_external())
    }

    pub fn is_read_only(&self) -> bool {
        self.effects.iter().all(|e| e.is_read_only())
    }

    pub fn contains(&self, effect: &Effect) -> bool {
        self.effects.contains(effect)
    }

    /// Join two effect sets (union).
    pub fn join(&self, other: &EffectSet) -> EffectSet {
        let mut merged = self.effects.clone();
        for eff in &other.effects {
            if !merged.contains(eff) {
                merged.push(eff.clone());
            }
        }
        EffectSet { effects: merged }
    }

    /// Returns true if all effects in self commute with all effects in other.
    pub fn commutes_with(&self, other: &EffectSet) -> bool {
        for a in &self.effects {
            for b in &other.effects {
                if !a.commutes_with(b) {
                    return false;
                }
            }
        }
        true
    }
}

impl fmt::Display for EffectSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[")?;
        for (i, effect) in self.effects.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "#arc.effect<{}>", effect)?;
        }
        write!(f, "]")
    }
}

/// Returns the inherent effects of a built-in operation kind.
pub fn inherent_effects(op_name: &str) -> EffectSet {
    match op_name {
        "arc.const" | "arc.add" | "arc.sub" | "arc.mul" | "arc.div" | "arc.icmp" | "arc.assume"
        | "arc.prove" | "arc.refine" => EffectSet::pure(),
        "arc.assert" => EffectSet::new(vec![Effect::Irreversible]),
        "arc.alloc" => EffectSet::new(vec![Effect::Allocate]),
        "arc.load" | "arc.load_elem" => EffectSet::new(vec![Effect::MemoryRead]),
        "arc.store" => EffectSet::new(vec![Effect::MemoryWrite]),
        "arc.call" => EffectSet::new(vec![Effect::ExternalCommunication]),
        "arc.require_approval" => EffectSet::new(vec![Effect::HumanApproval]),
        "arc.invoke" => EffectSet::new(vec![Effect::ExternalCommunication]),
        _ => EffectSet::pure(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_effects_commute_with_everything() {
        let pure = EffectSet::pure();
        let write = EffectSet::new(vec![Effect::MemoryWrite]);
        assert!(pure.commutes_with(&write));
        assert!(write.commutes_with(&pure));
    }

    #[test]
    fn reads_commute_with_reads() {
        let read1 = EffectSet::new(vec![Effect::MemoryRead]);
        let read2 = EffectSet::new(vec![Effect::DatabaseRead]);
        assert!(read1.commutes_with(&read2));
    }

    #[test]
    fn write_does_not_commute_with_write() {
        let w1 = EffectSet::new(vec![Effect::MemoryWrite]);
        let w2 = EffectSet::new(vec![Effect::MemoryWrite]);
        assert!(!w1.commutes_with(&w2));
    }

    #[test]
    fn read_does_not_commute_with_write() {
        let read = EffectSet::new(vec![Effect::MemoryRead]);
        let write = EffectSet::new(vec![Effect::MemoryWrite]);
        assert!(!read.commutes_with(&write));
    }

    #[test]
    fn external_effects_detected() {
        let net = EffectSet::new(vec![Effect::Network]);
        assert!(net.has_external_effects());
        let mem = EffectSet::new(vec![Effect::MemoryWrite]);
        assert!(!mem.has_external_effects());
    }

    #[test]
    fn join_merges_effects() {
        let a = EffectSet::new(vec![Effect::MemoryRead]);
        let b = EffectSet::new(vec![Effect::MemoryWrite, Effect::MemoryRead]);
        let joined = a.join(&b);
        assert_eq!(joined.effects().len(), 2);
    }

    #[test]
    fn effect_parse_roundtrip() {
        let names = [
            "pure",
            "memory.read",
            "memory.write",
            "allocate",
            "network",
            "irreversible",
        ];
        for name in names {
            let effect = Effect::parse(name).unwrap();
            assert_eq!(effect.as_str(), name);
        }
    }

    #[test]
    fn inherent_effects_for_builtins() {
        assert!(inherent_effects("arc.const").is_pure());
        assert!(inherent_effects("arc.add").is_pure());
        assert!(!inherent_effects("arc.alloc").is_pure());
        assert!(!inherent_effects("arc.store").is_pure());
        assert!(inherent_effects("arc.load").is_read_only());
    }
}
