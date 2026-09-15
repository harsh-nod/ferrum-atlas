use std::fmt;

#[derive(Debug)]
pub enum Error {
    InvalidQuery(String),
    NotFound,
    ContextMismatch,
    NotAuthorized,
    UnsupportedCapability,
    InvalidCursor,
    ExpiredCursor,
    BudgetExhausted,
    Store(atlas_store::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
}
impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidQuery(_) => "invalid_query",
            Self::NotFound => "not_found",
            Self::ContextMismatch => "context_mismatch",
            Self::NotAuthorized => "not_authorized",
            Self::UnsupportedCapability => "unsupported_capability",
            Self::InvalidCursor => "invalid_cursor",
            Self::ExpiredCursor => "expired_cursor",
            Self::BudgetExhausted => "budget_exhausted",
            Self::Store(error) => error.code(),
            Self::Io(_) | Self::Json(_) => "query_error",
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidQuery(message) => f.write_str(message),
            Self::Store(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
            Self::Json(error) => error.fmt(f),
            _ => f.write_str(self.code()),
        }
    }
}
impl std::error::Error for Error {}
impl From<atlas_store::Error> for Error {
    fn from(error: atlas_store::Error) -> Self {
        Self::Store(error)
    }
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
