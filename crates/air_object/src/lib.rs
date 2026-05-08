pub struct ObjectFile {
    pub format: String,
}

impl ObjectFile {
    pub fn new(format: impl Into<String>) -> Self {
        Self {
            format: format.into(),
        }
    }
}
