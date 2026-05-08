#[derive(Debug, Clone)]
pub struct ProofObligation {
    pub description: String,
}

pub fn discharge(obligation: &ProofObligation) -> bool {
    // Placeholder solver stub.
    !obligation.description.is_empty()
}
