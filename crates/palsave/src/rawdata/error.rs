use crate::gvas::error::GvasError;
use std::fmt;

#[derive(Debug)]
pub enum RawDataError {
    Gvas(GvasError),
    /// The decoder finished before consuming every byte, or consumed past the end —
    /// cheahjs's Python reference asserts `reader.eof()` at the same point, for the
    /// same reason: any RawData blob whose layout we don't fully understand should be
    /// left opaque rather than half-decoded. See `CLAUDE.md`'s "never guess" rule.
    NotExhausted {
        consumed: usize,
        total: usize,
    },
}

impl From<GvasError> for RawDataError {
    fn from(e: GvasError) -> Self {
        RawDataError::Gvas(e)
    }
}

impl fmt::Display for RawDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RawDataError::Gvas(e) => write!(f, "{e}"),
            RawDataError::NotExhausted { consumed, total } => {
                write!(
                    f,
                    "decoder consumed {consumed} of {total} bytes, expected exactly {total}"
                )
            }
        }
    }
}

impl std::error::Error for RawDataError {}
