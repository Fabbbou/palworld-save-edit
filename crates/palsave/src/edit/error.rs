use crate::gvas::GvasError;
use std::fmt;
use std::ops::Range;

#[derive(Debug)]
pub enum EditError {
    Gvas(GvasError),
    EmptyChain,
    /// A property chain passed to `replace_property_value` had an entry that doesn't
    /// physically contain the next one — the caller assembled it wrong (siblings
    /// instead of ancestors, or entries from different buffers).
    NotNested {
        outer: Range<usize>,
        inner: Range<usize>,
    },
    OverlappingSplices {
        first: Range<usize>,
        second: Range<usize>,
    },
    SpliceOutOfBounds {
        range: Range<usize>,
        source_len: usize,
    },
    /// A `size` fixup would move the field outside u32 range.
    SizeOutOfRange {
        offset: usize,
        old_size: u32,
        delta: i64,
    },
    /// The edited buffer didn't re-parse into an exact partition of itself, meaning
    /// some `size` field disagrees with the real byte layout. The buffer is not
    /// returned; see `edit::verify_reparses`.
    VerificationFailed,
}

impl From<GvasError> for EditError {
    fn from(e: GvasError) -> Self {
        EditError::Gvas(e)
    }
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::Gvas(e) => write!(f, "{e}"),
            EditError::EmptyChain => write!(f, "property chain was empty"),
            EditError::NotNested { outer, inner } => {
                write!(
                    f,
                    "property span {inner:?} is not contained in its declared parent {outer:?}"
                )
            }
            EditError::OverlappingSplices { first, second } => {
                write!(f, "overlapping splices: {first:?} and {second:?}")
            }
            EditError::SpliceOutOfBounds { range, source_len } => {
                write!(
                    f,
                    "splice range {range:?} exceeds source length {source_len}"
                )
            }
            EditError::SizeOutOfRange {
                offset,
                old_size,
                delta,
            } => {
                write!(
                    f,
                    "size fixup at offset {offset} ({old_size} + {delta}) is out of u32 range"
                )
            }
            EditError::VerificationFailed => {
                write!(
                    f,
                    "edited buffer did not re-parse into an exact partition of itself"
                )
            }
        }
    }
}

impl std::error::Error for EditError {}
