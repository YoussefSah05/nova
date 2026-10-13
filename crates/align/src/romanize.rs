//! Text → aligner tokens.
//!
//! The MMS forced-alignment model was trained on romanized text: its CTC
//! vocabulary is the 26 Latin letters plus the apostrophe. Words in other
//! scripts are romanized first, as `uroman` does for the model's training
//! data. Latin text only loses its accents ("été" → "ete"); Arabic is
//! transliterated letter by letter, with short-vowel marks kept when the
//! text carries them.
//!
//! A word that leaves no tokens (digits, symbols, an unsupported script) is
//! still a word: alignment skips it and the caller interpolates its time
//! from its neighbours.

use unicode_normalization::UnicodeNormalization;

/// The aligner's CTC vocabulary (`vocab.json` of the pinned MMS model).
pub const BLANK: u32 = 0;
const APOSTROPHE: u32 = 28;
const LETTERS: [u32; 26] = [
    4, 20, 23, 16, 6, 27, 17, 18, 5, 25, 14, 15, 13, 7, 8, 21, 29, 12, 11, 10, 9, 24, 22, 30, 19,
    26,
];
/// Width of the model's output layer.
pub const VOCAB_SIZE: usize = 31;

fn letter(c: char) -> Option<u32> {
    match c {
        'a'..='z' => Some(LETTERS[(c as u8 - b'a') as usize]),
        '\'' => Some(APOSTROPHE),
        _ => None,
    }
}

/// Romanization of one Arabic code point, following uroman's tables for the
/// letters that appear in Modern Standard and dialectal Arabic. `None` means
/// the character carries no sound of its own (tatweel, shadda, sukun,
/// punctuation).
fn arabic(c: char) -> Option<&'static str> {
    Some(match c {
        'ا' | 'أ' | 'آ' | 'ٱ' | 'ى' => "a",
        'إ' => "i",
        'ء' | 'ؤ' | 'ئ' | 'ع' => "'",
        'ب' => "b",
        'ت' | 'ة' => "t",
        'ث' => "th",
        'ج' => "j",
        'ح' | 'ه' => "h",
        'خ' => "kh",
        'د' | 'ض' => "d",
        'ذ' => "dh",
        'ر' => "r",
        'ز' | 'ظ' => "z",
        'س' | 'ص' => "s",
        'ش' => "sh",
        'ط' => "t",
        'غ' => "gh",
        'ف' | 'ڤ' => "f",
        'ق' => "q",
        'ك' | 'گ' => "k",
        'ل' => "l",
        'م' => "m",
        'ن' => "n",
        'و' => "w",
        'ي' | 'ی' => "y",
        'پ' => "p",
        'چ' => "ch",
        // Short vowels and tanwin, when the text is vocalized.
        '\u{064E}' => "a",
        '\u{064F}' => "u",
        '\u{0650}' => "i",
        '\u{064B}' => "an",
        '\u{064C}' => "un",
        '\u{064D}' => "in",
        _ => return None,
    })
}

/// Latin-letter romanization of `word`, lowercase, accents removed.
pub fn romanize(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    // Arabic letters are looked up whole: decomposition would split the
    // hamza off أ, إ, ؤ and ئ and lose their romanization.
    for c in word.chars().flat_map(|c| {
        let whole = arabic(c).map(|_| c);
        whole
            .into_iter()
            .chain(whole.is_none().then(|| c.nfkd()).into_iter().flatten())
    }) {
        if let Some(latin) = arabic(c) {
            out.push_str(latin);
            continue;
        }
        match c {
            // Combining marks left by NFKD (accents).
            '\u{0300}'..='\u{036F}' => {}
            'ß' => out.push_str("ss"),
            'æ' | 'Æ' => out.push_str("ae"),
            'œ' | 'Œ' => out.push_str("oe"),
            'ø' | 'Ø' => out.push('o'),
            'ł' | 'Ł' => out.push('l'),
            'đ' | 'Đ' => out.push('d'),
            '’' | 'ʼ' | '\'' => out.push('\''),
            c if c.is_ascii_alphabetic() => out.push(c.to_ascii_lowercase()),
            _ => {}
        }
    }
    out
}

/// The aligner tokens of `word` (possibly empty).
pub fn word_tokens(word: &str) -> Vec<u32> {
    let romanized = romanize(word);
    let tokens: Vec<u32> = romanized.chars().filter_map(letter).collect();
    // A lone apostrophe ("l'", a hamza) is not a sound worth aligning on.
    if tokens.iter().all(|&t| t == APOSTROPHE) {
        return Vec::new();
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_words_lose_case_and_accents_only() {
        assert_eq!(romanize("Été"), "ete");
        assert_eq!(romanize("L'accident,"), "l'accident");
        assert_eq!(romanize("cœur"), "coeur");
        assert_eq!(romanize("Straße"), "strasse");
        assert_eq!(romanize("aujourd’hui"), "aujourd'hui");
    }

    #[test]
    fn arabic_is_transliterated_letter_by_letter() {
        assert_eq!(romanize("مرحبا"), "mrhba");
        assert_eq!(romanize("الطقس"), "altqs");
        assert_eq!(romanize("شبه"), "shbh");
        // Vocalized text keeps its short vowels; shadda and tatweel vanish.
        assert_eq!(romanize("مُسماة"), "musmat");
        assert_eq!(romanize("ـمـ"), "m");
        // Hamza-bearing letters keep their own romanization.
        assert_eq!(romanize("إلى"), "ila");
        assert_eq!(romanize("أنا"), "ana");
        assert_eq!(romanize("سؤال"), "s'al");
        assert_eq!(romanize("قائمة"), "qa'mt");
    }

    #[test]
    fn tokens_use_the_model_vocabulary() {
        assert_eq!(word_tokens("ab"), vec![4, 20]);
        assert_eq!(word_tokens("z'"), vec![26, 28]);
        assert!(word_tokens("2026").is_empty());
        assert!(word_tokens("—").is_empty());
        assert!(word_tokens("l'").len() == 2);
        assert!(word_tokens("'").is_empty());
    }
}
