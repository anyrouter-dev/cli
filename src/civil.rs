//! UTC calendar math without a date crate (Howard Hinnant's algorithms).

/// Days since 1970-01-01 from a civil date.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil `(year, month, day)` for a count of days since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `YYYY-MM-DD` (UTC) for unix seconds.
pub fn ymd(epoch_secs: i64) -> String {
    let (y, m, d) = civil_from_days(epoch_secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Unix seconds for `YYYY-MM-DDTHH:MM:SS` (a `T` or a space), read as UTC.
/// Any fraction or zone suffix after the seconds is ignored; the API sends UTC.
pub fn parse_utc(stamp: &str) -> Option<i64> {
    let b = stamp.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b' ') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| stamp.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s)
}

/// Seconds since the unix epoch, now.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip_known_dates() {
        // (civil date, expected epoch seconds) pairs verified with `date -u`.
        for (y, mo, d, h, mi, s, epoch) in [
            (1970, 1, 1, 0, 0, 0, 0),
            (2026, 8, 21, 9, 2, 36, 1_787_302_956),
            (2000, 2, 29, 12, 0, 0, 951_825_600),
        ] {
            let computed = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s;
            assert_eq!(computed, epoch, "{y}-{mo}-{d} {h}:{mi}:{s}");
            assert_eq!(civil_from_days(epoch.div_euclid(86_400)), (y, mo, d));
        }
    }

    #[test]
    fn parse_utc_reads_iso_and_sqlite_stamps() {
        assert_eq!(parse_utc("2026-08-21T09:02:36Z"), Some(1_787_302_956));
        assert_eq!(parse_utc("2026-08-21 09:02:36"), Some(1_787_302_956));
        assert_eq!(parse_utc("2026-08-21T09:02:36.123Z"), Some(1_787_302_956));
        assert_eq!(parse_utc("yesterday"), None);
        assert_eq!(ymd(1_787_302_956), "2026-08-21");
    }
}
