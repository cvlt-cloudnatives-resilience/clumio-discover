use std::time::{SystemTime, UNIX_EPOCH};

use cd_model::Tags;

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

pub fn opt_bytes(n: Option<u64>) -> String {
    n.map(bytes).unwrap_or_else(|| "-".into())
}

pub fn tags(tags: &Tags) -> String {
    tags.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn when(ts: Option<&str>) -> String {
    match ts {
        Some(ts) => match (ts.get(..10), ts.get(11..16)) {
            (Some(date), Some(time)) => format!("{date} {time}"),
            _ => ts.to_string(),
        },
        None => "-".into(),
    }
}

pub fn pct(part: usize, total: usize) -> String {
    if total == 0 {
        "-".into()
    } else {
        format!("{:.0}%", part as f64 * 100.0 / total as f64)
    }
}

pub fn file_stamp(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (y, m, d) = civil(secs / 86_400);
    let rem = secs % 86_400;
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn civil(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn human_bytes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(8 * 1024 * 1024 * 1024), "8.0 GiB");
        assert_eq!(opt_bytes(None), "-");
    }

    #[test]
    fn file_stamp_is_utc_compact() {
        let t = UNIX_EPOCH + Duration::from_secs(1_792_728_306);
        assert_eq!(file_stamp(t), "20261023T040506");
        assert_eq!(file_stamp(UNIX_EPOCH), "19700101T000000");
    }

    #[test]
    fn short_timestamps() {
        assert_eq!(when(Some("2026-10-23T04:05:06Z")), "2026-10-23 04:05");
        assert_eq!(when(None), "-");
        assert_eq!(pct(1, 3), "33%");
        assert_eq!(pct(0, 0), "-");
    }
}
