//! Shared output helpers for the read commands (`logs`, `transactions`, ...).

use std::io::IsTerminal;

use serde_json::Value;

use crate::cmd::dispatch::USAGE;
use crate::term;

/// Plain table: aligned columns on a TTY, tab-separated when piped.
pub(crate) fn render_table(headers: &[&str], rows: &[Vec<String>], tty: bool) -> String {
    if !tty {
        return rows.iter().map(|r| r.join("\t") + "\n").collect();
    }
    let max = 48usize;
    let widths: Vec<usize> = (0..headers.len())
        .map(|i| {
            rows.iter()
                .map(|r| r[i].chars().count())
                .chain([headers[i].len()])
                .max()
                .unwrap_or(0)
                .min(max)
        })
        .collect();
    let fit = |s: &str, w: usize| -> String {
        if s.chars().count() > w {
            format!("{}…", s.chars().take(w - 1).collect::<String>())
        } else {
            format!("{s:<w$}")
        }
    };
    let head: Vec<String> = headers
        .iter()
        .zip(&widths)
        .map(|(h, w)| fit(&h.to_uppercase(), *w))
        .collect();
    let mut out = term::dim(head.join("  ").trim_end()) + "\n";
    for row in rows {
        let line: Vec<String> = row.iter().zip(&widths).map(|(c, w)| fit(c, *w)).collect();
        out += line.join("  ").trim_end();
        out.push('\n');
    }
    out
}

pub(crate) fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    print!(
        "{}",
        render_table(headers, rows, std::io::stdout().is_terminal())
    );
}

pub(crate) fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

/// `--limit` must be a whole number in `1..=max`.
pub(crate) fn parse_limit(raw: Option<&str>, max: u32) -> Result<Option<u32>, String> {
    let Some(raw) = raw else { return Ok(None) };
    match raw.parse::<u32>() {
        Ok(n) if (1..=max).contains(&n) => Ok(Some(n)),
        _ => Err(format!("{USAGE}--limit must be a number from 1 to {max}.")),
    }
}

pub(crate) fn text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "-".into(),
        Some(Value::String(s)) if s.is_empty() => "-".into(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// Parse `YYYY-MM-DD[T| ]HH:MM:SS[.fff][Z]` (UTC) into unix seconds.
pub(crate) fn parse_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (h, mi, se) = (n(11..13)?, n(14..16)?, n(17..19)?);
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + se)
}

/// `YYYY-MM-DD HH:MM` in the local zone (UTC where the OS has no zone info).
pub(crate) fn local_time(iso: &str) -> String {
    let Some(secs) = parse_utc(iso) else {
        return iso.to_string();
    };
    #[cfg(unix)]
    {
        let t = secs as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                tm.tm_year + 1900,
                tm.tm_mon + 1,
                tm.tm_mday,
                tm.tm_hour,
                tm.tm_min
            );
        }
    }
    iso.get(..16).unwrap_or(iso).replace('T', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utc_to_epoch() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_utc("2026-10-24T01:51:16.000Z"), Some(1_792_806_676));
        assert_eq!(parse_utc("nope"), None);
    }

    #[test]
    fn limit_bounds() {
        assert_eq!(parse_limit(None, 100), Ok(None));
        assert_eq!(parse_limit(Some("50"), 100), Ok(Some(50)));
        assert!(parse_limit(Some("0"), 100).is_err());
        assert!(parse_limit(Some("101"), 100).is_err());
        assert!(parse_limit(Some("x"), 100).is_err());
    }

    #[test]
    fn piped_table_is_tsv() {
        let rows = vec![vec!["a".to_string(), "b".to_string()]];
        assert_eq!(render_table(&["x", "y"], &rows, false), "a\tb\n");
    }
}
