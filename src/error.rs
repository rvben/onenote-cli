use std::fmt;

#[derive(Debug)]
pub struct Error {
    pub kind: &'static str,
    pub message: String,
    pub page_id: Option<String>,
}

pub const CONTRACTS: &[(&str, i32, &str)] = &[
    ("invalid_input", 2, "Invalid arguments"),
    (
        "config_error",
        2,
        "Invalid or unavailable profile configuration",
    ),
    ("read_only", 2, "The selected profile blocks writes"),
    (
        "unsupported",
        2,
        "Local access requires Windows or WSL; use SSH for a remote Windows desktop",
    ),
    ("not_found", 4, "The requested resource is unavailable"),
    (
        "desktop_error",
        5,
        "OneNote or its PowerShell bridge failed",
    ),
    (
        "conflict",
        6,
        "Page changed before the append; review before retrying",
    ),
    (
        "write_uncertain",
        7,
        "Write outcome is uncertain; inspect OneNote before retrying",
    ),
    ("unexpected_error", 1, "Local input/output failure"),
];

impl Error {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            page_id: None,
        }
    }
    pub fn with_page(mut self, id: &str) -> Self {
        self.page_id = Some(id.to_owned());
        self
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
