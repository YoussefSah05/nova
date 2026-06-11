//! BCP-47 ↔ NLLB-200 language-code mapping.
//!
//! NLLB-200 uses Flores-200 codes (`arb_Arab`, `fra_Latn`, `eng_Latn`, …), not
//! BCP-47. The mapping table is curated from `docs/reference/translate-config.md`;
//! we normalize an incoming tag to its base language (lowercasing, dropping any
//! region/script subtag) and accept both 2- and 3-letter forms because Whisper
//! emits ISO-639-1 (`ar`) while playlist track metadata often carries ISO-639-2
//! (`ara`/`fre`). The full Flores-200 list is large; we cover the languages a
//! user is realistically watching or reading in an IPTV catalog.

/// A language offered in the "Translate to" picker: a human label, the BCP-47
/// code persisted in settings, and the NLLB code handed to the model.
#[derive(Debug, Clone, Copy)]
pub struct TargetLanguage {
    pub label: &'static str,
    pub bcp47: &'static str,
    pub nllb: &'static str,
}

/// Curated target languages for the picker (display order). The same NLLB codes
/// back `to_nllb` for the source side.
pub const TARGET_LANGUAGES: &[TargetLanguage] = &[
    TargetLanguage { label: "English", bcp47: "en", nllb: "eng_Latn" },
    TargetLanguage { label: "French", bcp47: "fr", nllb: "fra_Latn" },
    TargetLanguage { label: "Arabic", bcp47: "ar", nllb: "arb_Arab" },
    TargetLanguage { label: "Spanish", bcp47: "es", nllb: "spa_Latn" },
    TargetLanguage { label: "German", bcp47: "de", nllb: "deu_Latn" },
    TargetLanguage { label: "Italian", bcp47: "it", nllb: "ita_Latn" },
    TargetLanguage { label: "Portuguese", bcp47: "pt", nllb: "por_Latn" },
    TargetLanguage { label: "Turkish", bcp47: "tr", nllb: "tur_Latn" },
    TargetLanguage { label: "Russian", bcp47: "ru", nllb: "rus_Cyrl" },
    TargetLanguage { label: "Hindi", bcp47: "hi", nllb: "hin_Deva" },
    TargetLanguage { label: "Chinese (Simplified)", bcp47: "zh", nllb: "zho_Hans" },
    TargetLanguage { label: "Japanese", bcp47: "ja", nllb: "jpn_Jpan" },
    TargetLanguage { label: "Korean", bcp47: "ko", nllb: "kor_Hang" },
    TargetLanguage { label: "Dutch", bcp47: "nl", nllb: "nld_Latn" },
];

/// Normalize a BCP-47 / ISO tag to its lowercase base language subtag
/// (`fr-FR` → `fr`, `ZH_Hans` → `zh`).
fn base(tag: &str) -> String {
    tag.trim()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Map a BCP-47 / ISO-639 code to its NLLB-200 code, or `None` if unsupported.
/// Accepts 2-letter (Whisper) and common 3-letter (track metadata) forms.
pub fn to_nllb(code: &str) -> Option<&'static str> {
    let b = base(code);
    let nllb = match b.as_str() {
        "en" | "eng" => "eng_Latn",
        "fr" | "fra" | "fre" => "fra_Latn",
        "ar" | "ara" => "arb_Arab",
        "es" | "spa" => "spa_Latn",
        "de" | "deu" | "ger" => "deu_Latn",
        "it" | "ita" => "ita_Latn",
        "pt" | "por" => "por_Latn",
        "tr" | "tur" => "tur_Latn",
        "ru" | "rus" => "rus_Cyrl",
        "hi" | "hin" => "hin_Deva",
        "zh" | "zho" | "chi" => "zho_Hans",
        "ja" | "jpn" => "jpn_Jpan",
        "ko" | "kor" => "kor_Hang",
        "nl" | "nld" | "dut" => "nld_Latn",
        _ => return None,
    };
    Some(nllb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_two_and_three_letter_and_regioned() {
        assert_eq!(to_nllb("ar"), Some("arb_Arab"));
        assert_eq!(to_nllb("ara"), Some("arb_Arab"));
        assert_eq!(to_nllb("fr-FR"), Some("fra_Latn"));
        assert_eq!(to_nllb("fre"), Some("fra_Latn"));
        assert_eq!(to_nllb("ZH_Hans"), Some("zho_Hans"));
        assert_eq!(to_nllb("en"), Some("eng_Latn"));
    }

    #[test]
    fn unknown_is_none() {
        assert_eq!(to_nllb(""), None);
        assert_eq!(to_nllb("xx"), None);
    }

    #[test]
    fn picker_codes_all_resolve() {
        for t in TARGET_LANGUAGES {
            assert_eq!(to_nllb(t.bcp47), Some(t.nllb), "{} should map", t.bcp47);
        }
    }
}
