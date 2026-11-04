//! Text decoding with BOM handling and conservative encoding detection.

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};

use crate::error::{Error, Result};

/// Result of decoding an input byte stream.
#[derive(Debug, Clone)]
pub struct DecodedText {
    pub text: String,
    pub encoding: &'static Encoding,
    /// True when invalid byte sequences had to be replaced.
    pub had_errors: bool,
    /// True when the bytes were not valid UTF-8 and a fallback encoding was
    /// selected without an explicit `--encoding`.
    pub used_fallback: bool,
}

impl DecodedText {
    pub fn encoding_name(&self) -> &'static str {
        self.encoding.name()
    }
}

/// Decode bytes as UTF-8/UTF-16 or, as a last resort, Windows-1252.
///
/// An explicit [`Encoding`] label (for example `latin1` or `utf-16le`) bypasses
/// detection. BOMs are stripped in every case.
pub fn decode_text(bytes: &[u8], encoding_label: Option<&str>) -> Result<DecodedText> {
    let (bom, body) = split_bom(bytes);

    if let Some(label) = encoding_label {
        let encoding = Encoding::for_label(label.trim().as_bytes()).ok_or_else(|| {
            Error::usage(format!(
                "unknown encoding '{label}' (try utf-8, utf-16le, utf-16be, windows-1252, or latin1)"
            ))
        })?;
        let (text, had_errors) = encoding.decode_without_bom_handling(body);
        return Ok(DecodedText {
            text: text.into_owned(),
            encoding,
            had_errors,
            used_fallback: false,
        });
    }

    if let Some(encoding) = bom {
        let (text, had_errors) = encoding.decode_without_bom_handling(body);
        return Ok(DecodedText {
            text: text.into_owned(),
            encoding,
            had_errors,
            used_fallback: false,
        });
    }

    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(DecodedText {
            text: text.to_owned(),
            encoding: UTF_8,
            had_errors: false,
            used_fallback: false,
        });
    }

    let encoding = guess_utf16(bytes).unwrap_or(WINDOWS_1252);
    let (text, had_errors) = encoding.decode_without_bom_handling(bytes);
    Ok(DecodedText {
        text: text.into_owned(),
        encoding,
        had_errors,
        used_fallback: true,
    })
}

fn split_bom(bytes: &[u8]) -> (Option<&'static Encoding>, &[u8]) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (Some(UTF_8), rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (Some(UTF_16LE), rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (Some(UTF_16BE), rest)
    } else {
        (None, bytes)
    }
}

/// Detect UTF-16 without a BOM by looking for a large share of NUL high bytes.
fn guess_utf16(bytes: &[u8]) -> Option<&'static Encoding> {
    let end = bytes.len().min(512) & !1;
    if end < 4 {
        return None;
    }

    let mut pairs = 0_u32;
    let mut le_nuls = 0_u32;
    let mut be_nuls = 0_u32;

    for pair in bytes[..end].as_chunks::<2>().0 {
        pairs += 1;
        if pair[1] == 0 {
            le_nuls += 1;
        }
        if pair[0] == 0 {
            be_nuls += 1;
        }
    }

    if le_nuls * 10 >= pairs * 8 {
        Some(UTF_16LE)
    } else if be_nuls * 10 >= pairs * 8 {
        Some(UTF_16BE)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_bom_is_stripped() {
        let decoded = decode_text(b"\xEF\xBB\xBFname\nAlice\n", None).unwrap();
        assert_eq!(decoded.text, "name\nAlice\n");
        assert_eq!(decoded.encoding, UTF_8);
        assert!(!decoded.had_errors);
    }

    #[test]
    fn utf16_bom_is_detected() {
        let bytes = b"\xFF\xFEn\x00a\x00m\x00e\x00\n\x00";
        let decoded = decode_text(bytes, None).unwrap();
        assert_eq!(decoded.text, "name\n");
        assert_eq!(decoded.encoding, UTF_16LE);
    }

    #[test]
    fn latin1_falls_back_to_windows_1252() {
        let decoded = decode_text(b"name\ncaf\xE9\n", None).unwrap();
        assert_eq!(decoded.text, "name\ncaf\u{e9}\n");
        assert_eq!(decoded.encoding, WINDOWS_1252);
        assert!(decoded.used_fallback);
        assert!(!decoded.had_errors);
    }

    #[test]
    fn explicit_encoding_wins() {
        let decoded = decode_text(b"caf\xE9", Some("latin1")).unwrap();
        assert_eq!(decoded.text, "caf\u{e9}");
        assert!(!decoded.used_fallback);
    }

    #[test]
    fn unknown_encoding_is_a_usage_error() {
        let error = decode_text(b"abc", Some("not-an-encoding")).unwrap_err();
        assert_eq!(error.exit_code(), crate::error::ExitCode::Usage);
    }
}
