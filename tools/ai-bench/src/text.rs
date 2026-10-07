//! Transcript normalization for WER/CER, shared by every language.
//!
//! In the spirit of the Open ASR Leaderboard's basic normalizer: NFKC, lower
//! case, punctuation and symbols become spaces, whitespace collapsed. Arabic
//! additionally drops diacritics and tatweel and unifies alef and alef maqsura
//! spellings, which otherwise dominate Arabic WER without changing meaning.
//! Ta marbuta is kept: it is a real orthographic distinction.

use unicode_normalization::UnicodeNormalization;

fn is_arabic_diacritic(c: char) -> bool {
    matches!(c, '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{0640}')
}

fn unify_arabic(c: char) -> char {
    match c {
        'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
        'ى' => 'ي',
        _ => c,
    }
}

/// Normalized text for scoring: words separated by single spaces.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.nfkc().flat_map(char::to_lowercase) {
        if is_arabic_diacritic(c) {
            continue;
        }
        let c = unify_arabic(c);
        out.push(if c.is_alphanumeric() { c } else { ' ' });
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn punctuation_case_and_spacing_do_not_count() {
        assert_eq!(normalize("Hello,  World!"), "hello world");
        assert_eq!(normalize("L'homme est là."), "l homme est là");
    }

    #[test]
    fn decomposed_accents_match_precomposed_ones() {
        assert_eq!(normalize("e\u{0301}te\u{0301}"), normalize("été"));
    }

    #[test]
    fn arabic_diacritics_and_letter_variants_are_unified() {
        // "مَدْرَسَةٌ" with harakat == bare form; أ/إ/آ → ا, ى → ي.
        assert_eq!(normalize("مَدْرَسَةٌ"), "مدرسة");
        assert_eq!(normalize("إلى أحمد"), "الي احمد");
        assert_eq!(normalize("كـتـاب"), "كتاب");
    }
}
