use std::fmt;

#[derive(Debug)]
pub enum ContainerError {
    TooShort {
        need: usize,
        have: usize,
    },
    PayloadOutOfBounds {
        start: usize,
        compressed_len: u32,
        file_len: usize,
    },
    UnknownMagic([u8; 3]),
    UnknownSaveType(u8),
    /// Oodle-compressed payload with a double zlib pass (save_type 0x32). The header
    /// has no field giving the intermediate buffer size Oodle would need to decode
    /// into, and no real PlM fixture has been seen with this save_type. Refusing to
    /// guess a buffer size rather than risk silent corruption. See ADR-001.md.
    UnsupportedOodleDoublePass,
    Zlib(std::io::Error),
    Oodle(oozextract::OozError),
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContainerError::TooShort { need, have } => {
                write!(
                    f,
                    "buffer too short: need at least {need} bytes, have {have}"
                )
            }
            ContainerError::PayloadOutOfBounds {
                start,
                compressed_len,
                file_len,
            } => write!(
                f,
                "compressed_len {compressed_len} at offset {start} exceeds file length {file_len}"
            ),
            ContainerError::UnknownMagic(m) => {
                write!(
                    f,
                    "unknown container magic {:?}",
                    String::from_utf8_lossy(m)
                )
            }
            ContainerError::UnknownSaveType(t) => write!(f, "unknown save_type 0x{t:02X}"),
            ContainerError::UnsupportedOodleDoublePass => {
                write!(
                    f,
                    "PlM with save_type 0x32 (double pass) is unverified and unsupported"
                )
            }
            ContainerError::Zlib(e) => write!(f, "zlib error: {e}"),
            ContainerError::Oodle(e) => write!(f, "oodle error: {e}"),
        }
    }
}

impl std::error::Error for ContainerError {}
