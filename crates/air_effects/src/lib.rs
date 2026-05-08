use air_ir::Type;

#[derive(Debug, Clone)]
pub struct EffectSet {
    effects: Vec<String>,
}

impl EffectSet {
    pub fn new(effects: Vec<String>) -> Self {
        Self { effects }
    }

    pub fn join(&self, other: &EffectSet) -> EffectSet {
        let mut merged = self.effects.clone();
        for eff in &other.effects {
            if !merged.contains(eff) {
                merged.push(eff.clone());
            }
        }
        EffectSet { effects: merged }
    }

    pub fn relates_to_type(&self, _ty: &Type) -> bool {
        !self.effects.is_empty()
    }
}
