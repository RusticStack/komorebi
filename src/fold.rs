//! Search folding: lower case without accents, so "joao" finds "João" and
//! "STRASSE" finds "Straße". A small table covers the Latin letters mail
//! uses; anything else is only lower-cased. Combining marks (decomposed
//! accents) are dropped. Pure Rust, no Unicode tables beyond `std`'s.

use std::borrow::Cow;

/// `s` lower-cased with Latin accents removed. Borrows `s` when it is
/// already folded (lower-case ASCII, most header text).
pub fn fold(s: &str) -> Cow<'_, str> {
    if !s.bytes().any(|b| b.is_ascii_uppercase() || !b.is_ascii()) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
            continue;
        }
        for c in c.to_lowercase() {
            match base(c) {
                Some(b) => out.push_str(b),
                None if is_combining(c) => {}
                None => out.push(c),
            }
        }
    }
    Cow::Owned(out)
}

/// Combining diacritical marks (U+0300 to U+036F): an accent written as a
/// second character after its letter.
fn is_combining(c: char) -> bool {
    ('\u{300}'..='\u{36f}').contains(&c)
}

/// The plain letters of a lower-case accented Latin letter.
fn base(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĳ' => "ij",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' | 'ț' => "t",
        'þ' => "th",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_and_accents() {
        assert_eq!(fold("JOSÉ"), "jose");
        assert_eq!(fold("Ação"), "acao");
        assert_eq!(fold("Straße"), "strasse");
        assert_eq!(fold("STRASSE"), "strasse");
        assert_eq!(fold("João Gonçalves"), "joao goncalves");
        assert_eq!(fold("Œuvre, Ærø, Łódź"), "oeuvre, aero, lodz");
        // Upper-case letters that lower-case to an accented one.
        assert_eq!(fold("ÇÃÕÑ ẞ"), "caon ss");
        // A decomposed accent (e + U+0301) folds like the composed one.
        assert_eq!(fold("Jose\u{301}"), "jose");
        // İ lower-cases to i plus a combining dot.
        assert_eq!(fold("İstanbul"), "istanbul");
    }

    #[test]
    fn borrows_folded_text() {
        assert!(matches!(fold("invoice 2041"), Cow::Borrowed(_)));
        assert!(matches!(fold(""), Cow::Borrowed(_)));
        assert!(matches!(fold("Invoice"), Cow::Owned(_)));
    }

    #[test]
    fn other_scripts_only_lower_case() {
        assert_eq!(fold("ΣΟΦΙΑ"), "σοφια");
        assert_eq!(fold("Привет"), "привет");
        assert_eq!(fold("東京 🙂"), "東京 🙂");
        assert_eq!(
            fold("a\u{0}\u{1f}\u{feff}\u{200d}b"),
            "a\u{0}\u{1f}\u{feff}\u{200d}b"
        );
        // Stray combining marks on their own are dropped, never a panic.
        assert_eq!(fold("\u{301}\u{36f}x\u{300}"), "x");
    }
}
