use air_ir::Type;

pub fn normalize_type(repr: &str) -> Type {
    Type::new(repr.trim())
}
