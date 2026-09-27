//! The display-name policy: which of a title's names the user sees.
//!
//! Precedence, strongest first:
//!
//! 1. **User correction**: an explicit rename by the user.
//! 2. **Trusted metadata**: a *confident* TMDB match's title, in the profile's
//!    metadata language.
//! 3. **Parsed provider title**: `clean::parse_source_title`'s title (prefixes,
//!    tags and numbering removed).
//! 4. **Raw provider title**: last resort, so a card is never blank.
//!
//! Low-confidence and unresolved matches carry no trusted title, so they fall
//! through to the parsed title: a guess never becomes the display name. SQL
//! read paths apply the same order with `COALESCE(<tmdb title>, <parsed>,
//! <raw>)`. Keep them in sync with [`display_title`] when this changes.
//! A display title is presentation only: identity never depends on it.

/// Version of the precedence rules above.
pub const NAMING_POLICY_VERSION: u32 = 1;

/// Every name a title may have. Empty or whitespace-only values count as
/// absent.
#[derive(Debug, Clone, Copy, Default)]
pub struct DisplayNameSources<'a> {
    /// Reserved for user corrections (no correction store exists yet).
    pub user: Option<&'a str>,
    /// Confident TMDB title (never set from an unresolved or weak match).
    pub trusted_metadata: Option<&'a str>,
    pub parsed: Option<&'a str>,
    pub raw: &'a str,
}

/// The title to display, by the policy's precedence.
pub fn display_title(sources: DisplayNameSources<'_>) -> String {
    [sources.user, sources.trusted_metadata, sources.parsed]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| sources.raw.trim())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names<'a>(
        user: Option<&'a str>,
        tmdb: Option<&'a str>,
        parsed: Option<&'a str>,
        raw: &'a str,
    ) -> DisplayNameSources<'a> {
        DisplayNameSources {
            user,
            trusted_metadata: tmdb,
            parsed,
            raw,
        }
    }

    #[test]
    fn each_source_wins_over_the_weaker_ones() {
        let raw = "FR | BREAKING.BAD.S03E07.1080P";
        assert_eq!(
            display_title(names(
                Some("My Show"),
                Some("Breaking Bad"),
                Some("BREAKING BAD"),
                raw
            )),
            "My Show"
        );
        assert_eq!(
            display_title(names(None, Some("Breaking Bad"), Some("BREAKING BAD"), raw)),
            "Breaking Bad"
        );
        assert_eq!(
            display_title(names(None, None, Some("BREAKING BAD"), raw)),
            "BREAKING BAD"
        );
        assert_eq!(display_title(names(None, None, None, raw)), raw);
    }

    #[test]
    fn blank_names_never_win() {
        assert_eq!(
            display_title(names(Some("  "), Some(""), Some("Heat"), "Heat FHD")),
            "Heat"
        );
    }
}
