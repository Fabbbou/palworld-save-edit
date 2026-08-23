use std::fmt;

#[derive(Debug)]
pub enum GvasError {
    UnexpectedEof {
        need: usize,
        at: usize,
        have: usize,
    },
    /// Property tags with complete type names (engine >= 5.4) are a different,
    /// tree-structured wire format we haven't seen a real fixture for. Refusing to
    /// guess at it rather than risk silently misreading every property after this
    /// point. See ADR-001.md / gvas/mod.rs module docs.
    UnsupportedPropertyTagFormat {
        engine_major: u16,
        engine_minor: u16,
    },
    UnknownPropertyType {
        name: String,
        at: usize,
    },
    /// A layout walk ended somewhere other than the property's declared end. Only
    /// raised by `value::map_layout`, where landing off by even one byte would mean
    /// slicing an entry in half — so it fails loudly instead of returning spans that
    /// look plausible.
    TrailingBytes {
        at: usize,
        expected: usize,
    },
}

impl fmt::Display for GvasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GvasError::UnexpectedEof { need, at, have } => {
                write!(
                    f,
                    "unexpected EOF at offset {at}: need {need} more bytes, have {have}"
                )
            }
            GvasError::UnsupportedPropertyTagFormat {
                engine_major,
                engine_minor,
            } => write!(
                f,
                "engine version {engine_major}.{engine_minor} uses the complete-type-name \
                 property tag format (>= 5.4), which isn't implemented yet"
            ),
            GvasError::UnknownPropertyType { name, at } => {
                write!(f, "unknown property type {name:?} at offset {at}")
            }
            GvasError::TrailingBytes { at, expected } => {
                write!(
                    f,
                    "layout walk ended at offset {at} but the property declares it ends \
                     at {expected}"
                )
            }
        }
    }
}

impl std::error::Error for GvasError {}
