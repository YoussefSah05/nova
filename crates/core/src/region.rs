//! Region bucketing for live channels. IPTV providers tag channels with a short
//! country code (the parser's `country_prefix`, e.g. `US`, `FR`, `AR`); the browse
//! UI groups by a coarser **region** so the filter has a handful of options instead
//! of dozens of countries. Curated map, ASCII uppercase keys.

/// Map a 2-letter country code to a coarse region. `None` when unknown (caller
/// leaves the channel region-less / under "Other").
pub fn region_of(country: &str) -> Option<&'static str> {
    let code = country.trim().to_ascii_uppercase();
    Some(match code.as_str() {
        // MENA (Middle East & North Africa). Note: IPTV providers use "AR" for
        // Arabic content far more often than Argentina, so it's bucketed here.
        "AR" | "AE" | "SA" | "QA" | "KW" | "BH" | "OM" | "IQ" | "JO" | "LB" | "SY" | "YE"
        | "PS" | "IL" | "IR" | "EG" | "DZ" | "MA" | "TN" | "LY" | "SD" | "MR" => "MENA",
        // Europe
        "GB" | "UK" | "IE" | "FR" | "DE" | "ES" | "PT" | "IT" | "NL" | "BE" | "LU" | "CH"
        | "AT" | "SE" | "NO" | "DK" | "FI" | "IS" | "PL" | "CZ" | "SK" | "HU" | "RO" | "BG"
        | "GR" | "HR" | "RS" | "SI" | "BA" | "MK" | "AL" | "UA" | "RU" | "BY" | "LT" | "LV"
        | "EE" | "TR" | "CY" | "MT" => "Europe",
        // North America
        "US" | "CA" => "North America",
        // Latin America
        "MX" | "BR" | "CL" | "CO" | "PE" | "VE" | "EC" | "BO" | "PY" | "UY" | "GT" | "CR"
        | "PA" | "DO" | "CU" | "HN" | "SV" | "NI" | "PR" => "Latin America",
        // Asia
        "CN" | "JP" | "KR" | "IN" | "PK" | "BD" | "LK" | "ID" | "MY" | "SG" | "TH" | "VN"
        | "PH" | "TW" | "HK" | "AF" | "NP" | "KZ" | "UZ" => "Asia",
        // Africa (sub-Saharan)
        "NG" | "ZA" | "KE" | "GH" | "ET" | "TZ" | "UG" | "SN" | "CI" | "CM" | "AO" | "ZW"
        | "ZM" => "Africa",
        // Oceania
        "AU" | "NZ" | "FJ" | "PG" => "Oceania",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_known_codes() {
        assert_eq!(region_of("AR"), Some("MENA")); // IPTV "AR" = Arabic
        assert_eq!(region_of("fr"), Some("Europe"));
        assert_eq!(region_of("US"), Some("North America"));
        assert_eq!(region_of("JP"), Some("Asia"));
        assert_eq!(region_of("ZZ"), None);
    }
}
