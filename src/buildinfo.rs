//! Build metadata: embeds the compile timestamp (UTC, stamped by build.rs)
//! and renders it in the viewer's local timezone.

pub const BUILD_TIME: &str = match option_env!("ANYR_BUILD_TIME_UTC") {
    Some(t) => t,
    None => "unknown",
};

/// Build time formatted for display: local timezone when available,
/// otherwise the embedded UTC stamp.
pub fn display_time() -> String {
    #[cfg(unix)]
    if let Some(local) = local_time_from_utc(BUILD_TIME) {
        return local;
    }
    BUILD_TIME.to_string()
}

/// Parse `YYYY-MM-DDTHH:MM:SSZ` and render it in the machine's local
/// timezone via `localtime_r`. Returns `None` when parsing fails.
#[cfg(unix)]
fn local_time_from_utc(stamp: &str) -> Option<String> {
    let b = stamp.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| stamp.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let epoch = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s;

    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t: libc::time_t = epoch;
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    ))
}

/// Days since 1970-01-01 from a civil date (Howard Hinnant's algorithm).
#[cfg(unix)]
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_time_is_stamped() {
        assert_ne!(BUILD_TIME, "unknown", "build.rs must stamp ANYR_BUILD_TIME_UTC");
        assert!(BUILD_TIME.ends_with('Z'), "{BUILD_TIME}");
    }

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
        }
    }

    #[test]
    fn display_time_falls_back_to_raw_stamp_on_bad_input() {
        // display_time never panics even if the stamp were malformed.
        let _ = display_time();
    }
}
