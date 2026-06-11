//! FTS5 query building for the `transcript_fts` index (Phase 8).
//!
//! The only job here is turning raw user input into a **safe** FTS5 MATCH
//! expression. FTS5 treats `" : ( ) { } * ^ -` and bareword `AND/OR/NOT/NEAR` as
//! operators, so unescaped input is both an injection risk and a syntax-error
//! risk. We sidestep all of it by quoting the whole input as a single phrase
//! (doubling any embedded quote) and appending `*` so the last word
//! prefix-matches — i.e. live typeahead: `prof` → `"prof"*` matches "profit".
//! The storage layer binds the result to `text MATCH ?`.

/// Build a safe FTS5 phrase-prefix MATCH expression from raw user input.
/// Returns an empty string for blank input — callers must treat that as "no
/// query" and skip the search (an empty MATCH is a syntax error in FTS5).
pub fn fts_match(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let escaped = trimmed.replace('"', "\"\"");
    format!("\"{escaped}\"*")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_and_prefixes() {
        assert_eq!(fts_match("profit"), "\"profit\"*");
        assert_eq!(fts_match("profit margin"), "\"profit margin\"*");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(fts_match("   profit margin  "), "\"profit margin\"*");
    }

    #[test]
    fn blank_is_empty() {
        assert_eq!(fts_match(""), "");
        assert_eq!(fts_match("    "), "");
    }

    #[test]
    fn neutralizes_operators_and_quotes() {
        // Embedded quotes are doubled; operators become literal inside the phrase.
        assert_eq!(fts_match("say \"hi\""), "\"say \"\"hi\"\"\"*");
        assert_eq!(fts_match("a OR b) (c"), "\"a OR b) (c\"*");
        assert_eq!(fts_match("*"), "\"*\"*");
    }
}
