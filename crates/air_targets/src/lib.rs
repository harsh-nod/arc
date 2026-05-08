pub struct TargetDescription {
    pub name: &'static str,
    pub pointer_size: u8,
}

pub fn builtin_targets() -> Vec<TargetDescription> {
    vec![
        TargetDescription {
            name: "x86_64-air-linux",
            pointer_size: 8,
        },
        TargetDescription {
            name: "wasm32-air",
            pointer_size: 4,
        },
    ]
}
