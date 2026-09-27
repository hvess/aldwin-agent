//! Keystrokes, as bytes.
//!
//! The harness chooses these, which is the one place it stands in for the
//! terminal rather than using it: foot's own key *encoding* is not exercised
//! by anything here, so a defect in that layer — `40cb6b1` was one — stays
//! invisible. `wtype` is the answer if that class recurs; the two compose.
//!
//! Everything else in the input path is real: what the app writes reaches
//! foot, and foot's replies to the app's capability queries reach the app.

use crate::{Error, Result};

/// Turns a scene's comma-separated key spec — `Down, "ok", Enter` — into the
/// bytes each key writes, one entry per key.
///
/// # Errors
///
/// [`Error::Scene`] when a token is neither a known key name nor a quoted
/// literal; the message names the token and the keys that are known.
pub fn parse(spec: &str) -> Result<Vec<Vec<u8>>> {
    tokenise(spec)
        .into_iter()
        .map(|token| match token.as_str() {
            "Up" => Ok(b"\x1b[A".to_vec()),
            "Down" => Ok(b"\x1b[B".to_vec()),
            // The review's keyboard way to a selection (ADR 0010). Shift
            // keeps the legacy form under the Kitty protocol's
            // "disambiguate" flag, the only one the app pushes.
            "ShiftDown" => Ok(b"\x1b[1;2B".to_vec()),
            "Right" => Ok(b"\x1b[C".to_vec()),
            "Left" => Ok(b"\x1b[D".to_vec()),
            "Enter" => Ok(b"\r".to_vec()),
            "Tab" => Ok(b"\t".to_vec()),
            "Esc" => Ok(b"\x1b".to_vec()),
            "Space" => Ok(b" ".to_vec()),
            "Backspace" => Ok(b"\x7f".to_vec()),
            other if other.starts_with('"') && other.ends_with('"') && other.len() >= 2 => {
                // `\e` is an escape, so a raw sequence can be sent verbatim —
                // needed to ask what a key looks like in a protocol the app
                // negotiated with the terminal rather than with us.
                Ok(other[1..other.len() - 1].replace("\\e", "\x1b").into_bytes())
            }
            other => Err(Error::Scene(format!("unknown key {other:?} (Up|Down|ShiftDown|Left|Right|Enter|Tab|Esc|Space|Backspace|\"literal\")"))),
        })
        .collect()
}

/// Split on commas *outside* quotes.
///
/// Splitting first and recognising quotes afterwards tore `"hello, world"`
/// into two tokens and then complained that `"hello` was not a key — an error
/// pointing at the wrong thing, for a scene that had done nothing wrong.
fn tokenise(spec: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in spec.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            ',' if !quoted => {
                let token = current.trim().to_string();
                if !token.is_empty() {
                    tokens.push(token);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let token = current.trim().to_string();
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_literal_may_contain_a_comma() {
        let keys = parse("\"hello, world\",Enter").expect("a quoted comma is not a separator");
        assert_eq!(keys, vec![b"hello, world".to_vec(), b"\r".to_vec()]);
    }

    #[test]
    fn named_keys_and_literals_mix() {
        let keys = parse("Down, \"ok\" ,Enter").expect("parses");
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0], b"\x1b[B".to_vec());
    }

    #[test]
    fn an_escape_reaches_the_app_as_one_byte() {
        assert_eq!(parse("\"\\e[9u\"").unwrap(), vec![b"\x1b[9u".to_vec()]);
    }

    #[test]
    fn an_unknown_name_names_itself_in_the_error() {
        let unknown = parse("Meta").unwrap_err();
        assert!(matches!(unknown, Error::Scene(_)));
        assert!(unknown.to_string().contains("Meta"));
    }
}
