use std::fmt;

#[derive(Debug)]
pub struct Error {
    pub kind: &'static str,
    pub message: String,
}

pub const CONTRACTS: &[(&str, i32, &str)] = &[
    ("invalid_input", 2, "Invalid arguments"),
    (
        "unsupported",
        2,
        "Requires Windows or WSL and Windows OneNote desktop",
    ),
    ("not_found", 4, "The requested resource is unavailable"),
    (
        "desktop_error",
        5,
        "OneNote or its PowerShell bridge failed",
    ),
    ("unexpected_error", 1, "Local input/output failure"),
];

impl Error {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_input", message)
    }
    pub fn desktop(message: impl Into<String>) -> Self {
        Self::new("desktop_error", message)
    }
    pub fn code(&self) -> i32 {
        CONTRACTS
            .iter()
            .find(|c| c.0 == self.kind)
            .map_or(1, |c| c.1)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new("unexpected_error", e.to_string())
    }
}
