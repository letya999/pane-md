use std::{fmt, str::Utf8Error};

#[derive(Debug)]
pub enum DecodeError {
    Utf8(Utf8Error),
    Nul,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(error) => error.fmt(formatter),
            Self::Nul => formatter.write_str("the document contains a null character"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Returns the presence of a UTF-8 BOM and the text without it.
pub fn decode(bytes: &[u8]) -> Result<(bool, &str), DecodeError> {
    let body = bytes.strip_prefix(b"\xef\xbb\xbf");
    let text = std::str::from_utf8(body.unwrap_or(bytes)).map_err(DecodeError::Utf8)?;
    if text.contains('\0') {
        return Err(DecodeError::Nul);
    }
    Ok((body.is_some(), text))
}
