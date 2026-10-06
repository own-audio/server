// SPDX-License-Identifier: AGPL-3.0-or-later
//! Text intake for `multipart/form-data` requests.
//!
//! Kept here rather than in the domain modules because `audiobooks` and `music`
//! both parse uploads and their two hand-rolled copies had already drifted into
//! the same bug.

use crate::auth::error::AuthError;

/// Reads a form field as text, trimmed.
///
/// **Never decode a form field lossily.** `Field::text()` runs the bytes through
/// `encoding_rs` as UTF-8 and replaces every invalid byte with U+FFFD, so a
/// Windows-1250 title arrives as `P\u{fffd}\u{fffd}li\u{fffd}` and is stored that
/// way — the original bytes are gone and no later repair is possible. A real
/// audiobook reached the database with a destroyed Czech title before this
/// existed.
pub async fn read_text_field(
    field: axum::extract::multipart::Field<'_>,
) -> Result<String, AuthError> {
    let bytes = field
        .bytes()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid form field: {e}")))?;

    decode_form_text(&bytes)
        .map(|text| text.trim().to_string())
        .ok_or_else(|| AuthError::BadRequest("form field is not valid text".to_string()))
}

/// UTF-8, then Windows-1250 for the older Central European exports this product
/// sees, then give up. Returning `None` rather than falling back to a lossy
/// decode is the whole point: garbled-but-accepted is worse than rejected.
///
/// CP-1250 maps almost every byte, so "it decoded" proves very little on its
/// own — UTF-16 text runs through it cleanly and comes out interleaved with
/// NULs. Control characters are therefore the tell that the guess was wrong.
fn decode_form_text(bytes: &[u8]) -> Option<String> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some(text.to_string());
    }

    let (decoded, _, had_errors) = encoding_rs::WINDOWS_1250.decode(bytes);
    if had_errors || decoded.chars().any(is_unexpected_control) {
        return None;
    }
    Some(decoded.into_owned())
}

/// Tab and the newline pair are legitimate in a multi-line field such as a
/// description; nothing else below U+0020 is.
fn is_unexpected_control(ch: char) -> bool {
    ch.is_control() && !matches!(ch, '\t' | '\n' | '\r')
}

#[cfg(test)]
mod tests {
    use super::decode_form_text;

    const TITLE: &str = "Příliš žluťoučký kůň";

    #[test]
    fn utf8_passes_through() {
        assert_eq!(decode_form_text(TITLE.as_bytes()).as_deref(), Some(TITLE));
    }

    #[test]
    fn windows_1250_is_recovered() {
        let (bytes, _, had_errors) = encoding_rs::WINDOWS_1250.encode(TITLE);
        assert!(!had_errors, "test input must be representable in CP-1250");
        assert_eq!(decode_form_text(&bytes).as_deref(), Some(TITLE));
    }

    /// The regression: this input used to become `P\u{fffd}\u{fffd}li\u{fffd} …`.
    #[test]
    fn no_output_ever_contains_replacement_characters() {
        let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode(TITLE);
        let decoded = decode_form_text(&bytes).expect("CP-1250 decodes");
        assert!(!decoded.contains('\u{fffd}'));
    }

    /// UTF-16LE is invalid UTF-8 and CP-1250 "succeeds" on it, so only the
    /// interleaved NULs give it away.
    #[test]
    fn undecodable_input_is_rejected_not_mangled() {
        let utf16: Vec<u8> = TITLE.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode_form_text(&utf16), None);
    }

    #[test]
    fn multi_line_description_survives() {
        let text = "Řádek jedna\nŘádek dvě";
        let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode(text);
        assert_eq!(decode_form_text(&bytes).as_deref(), Some(text));
    }
}
