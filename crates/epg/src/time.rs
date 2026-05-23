//! XMLTV date → UTC epoch conversion.
//!
//! XMLTV uses `YYYYMMDDHHMMSS ±HHMM` (e.g. `20240101130000 -0500`). The
//! timezone offset is optional; when absent we treat it as UTC per
//! `docs/reference/xmltv-epg.md`.

/// Parse an XMLTV datetime string into UTC epoch seconds. Returns `None` for
/// any malformed input — the caller should skip the programme.
pub fn xmltv_to_epoch(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 14 {
        return None;
    }
    let y: i32 = s.get(0..4)?.parse().ok()?;
    let mo: u32 = s.get(4..6)?.parse().ok()?;
    let d: u32 = s.get(6..8)?.parse().ok()?;
    let h: u32 = s.get(8..10)?.parse().ok()?;
    let mi: u32 = s.get(10..12)?.parse().ok()?;
    let se: u32 = s.get(12..14)?.parse().ok()?;

    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h >= 24 || mi >= 60 || se >= 60 {
        return None;
    }

    let offset_seconds: i64 = match s.get(14..) {
        Some(rest) => parse_offset(rest).unwrap_or(0),
        None => 0,
    };

    let days = days_from_civil(y, mo as i32, d as i32);
    let local = days * 86400 + (h as i64) * 3600 + (mi as i64) * 60 + (se as i64);
    Some(local - offset_seconds)
}

fn parse_offset(rest: &str) -> Option<i64> {
    let rest = rest.trim_start();
    let bytes = rest.as_bytes();
    if bytes.len() < 5 {
        return None;
    }
    let sign: i64 = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let oh: i64 = rest.get(1..3)?.parse().ok()?;
    let om: i64 = rest.get(3..5)?.parse().ok()?;
    Some(sign * (oh * 3600 + om * 60))
}

/// Howard Hinnant's "days from civil date" algorithm — returns the number of
/// days since 1970-01-01 (negative for earlier dates). Valid for any proleptic
/// Gregorian date; no leap-second handling needed for EPG use.
fn days_from_civil(y: i32, m: i32, d: i32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = (y - era * 400) as i64; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era as i64 * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch_zero() {
        // 1970-01-01 00:00:00 UTC = 0
        assert_eq!(xmltv_to_epoch("19700101000000 +0000"), Some(0));
        // Without offset, treated as UTC.
        assert_eq!(xmltv_to_epoch("19700101000000"), Some(0));
    }

    #[test]
    fn known_date_2024() {
        // 2024-01-01 12:00:00 UTC = 1704110400
        assert_eq!(xmltv_to_epoch("20240101120000 +0000"), Some(1704110400));
    }

    #[test]
    fn positive_offset_subtracts() {
        // 2024-01-01 13:00:00 +0100 = 12:00:00 UTC
        assert_eq!(xmltv_to_epoch("20240101130000 +0100"), Some(1704110400));
    }

    #[test]
    fn negative_offset_adds() {
        // 2024-01-01 07:00:00 -0500 = 12:00:00 UTC
        assert_eq!(xmltv_to_epoch("20240101070000 -0500"), Some(1704110400));
    }

    #[test]
    fn rejects_invalid() {
        assert!(xmltv_to_epoch("").is_none());
        assert!(xmltv_to_epoch("20241301000000").is_none()); // month 13
        assert!(xmltv_to_epoch("20240101250000").is_none()); // hour 25
        assert!(xmltv_to_epoch("not a date").is_none());
    }
}
