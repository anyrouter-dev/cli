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
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| stamp.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let epoch = crate::civil::days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_time_is_stamped() {
        assert_ne!(
            BUILD_TIME, "unknown",
            "build.rs must stamp ANYR_BUILD_TIME_UTC"
        );
        assert!(BUILD_TIME.ends_with('Z'), "{BUILD_TIME}");
    }

    #[test]
    fn display_time_falls_back_to_raw_stamp_on_bad_input() {
        // display_time never panics even if the stamp were malformed.
        let _ = display_time();
    }
}
