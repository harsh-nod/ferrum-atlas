use std::fmt;

#[derive(Debug)]
pub enum Error {
    Invalid(String),
    Conflict,
    UnknownSnapshot,
    Unavailable(String),
    UnsupportedVersion(u32),
    Io(std::io::Error),
    Sql(rusqlite::Error),
    Json(serde_json::Error),
}
impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_batch",
            Self::Conflict => "publication_conflict",
            Self::UnknownSnapshot => "unknown_snapshot",
            Self::Unavailable(_) | Self::Io(_) => "unavailable_shard",
            Self::UnsupportedVersion(_) => "unsupported_schema",
            Self::Sql(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::OperationInterrupted =>
            {
                "budget_exhausted"
            }
            Self::Sql(_) | Self::Json(_) => "store_error",
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) | Self::Unavailable(message) => f.write_str(message),
            Self::Conflict => f.write_str("workspace head changed before publication"),
            Self::UnknownSnapshot => f.write_str("unknown snapshot"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported storage schema {v}"),
            Self::Io(e) => e.fmt(f),
            Self::Sql(e) => e.fmt(f),
            Self::Json(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
macro_rules! convert {
    ($source:ty, $variant:ident) => {
        impl From<$source> for Error {
            fn from(error: $source) -> Self {
                Self::$variant(error)
            }
        }
    };
}
convert!(std::io::Error, Io);
convert!(rusqlite::Error, Sql);
convert!(serde_json::Error, Json);
pub type Result<T> = std::result::Result<T, Error>;
