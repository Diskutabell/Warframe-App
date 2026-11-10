//! Wall-clock time: epoch seconds and UTC calendar dates, without a date library.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the epoch, the unit every stored timestamp uses.
pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// (year, month, day, hour, minute, second) in UTC.
fn utc(epoch: f64) -> (i64, i64, i64, i64, i64, i64) {
    let secs = epoch.max(0.0) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// "20261009-153012": for file names that sort by time.
pub fn stamp(epoch: f64) -> String {
    let (y, mo, d, h, mi, s) = utc(epoch);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// "2026-10-09T15:30:12Z", the form the worldstate API and the UI use.
pub fn iso(epoch: f64) -> String {
    let (y, mo, d, h, mi, s) = utc(epoch);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Epoch seconds from an ISO-8601 UTC time as the worldstate API writes it
/// ("2026-10-16T13:00:00.000Z", fractional seconds optional).
pub fn parse_iso(s: &str) -> Option<f64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, mo, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.splitn(3, ':');
    let (h, mi) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let sec: f64 = t.next()?.parse().ok()?;
    // Howard Hinnant's days_from_civil.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86400 + h * 3600 + mi * 60) as f64 + sec)
}

/// Epoch seconds from the form EE.log's header writes the time in ("Tue Nov 10 00:07:35 2026"),
/// read as UTC.
pub fn parse_ctime(s: &str) -> Option<f64> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let parts: Vec<&str> = s.split_whitespace().collect();
    let [_weekday, month, day, time, year] = parts.as_slice() else { return None };
    let month = MONTHS.iter().position(|m| m == month)? + 1;
    let day: u32 = day.parse().ok()?;
    parse_iso(&format!("{year}-{month:02}-{day:02}T{time}Z"))
}

/// Warframe's daily reset: the next 00:00 UTC.
pub fn next_daily_reset(epoch: f64) -> f64 {
    ((epoch / 86400.0).floor() + 1.0) * 86400.0
}

/// Warframe's weekly reset: the next Monday 00:00 UTC. (The epoch began on a Thursday.)
pub fn next_weekly_reset(epoch: f64) -> f64 {
    let day = (epoch / 86400.0).floor() as i64;
    let weekday = (day + 3).rem_euclid(7); // 0 = Monday
    (day + 7 - weekday) as f64 * 86400.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar() {
        assert_eq!(stamp(0.0), "19700101-000000");
        assert_eq!(stamp(1_791_032_607.4), "20261003-130327");
        assert_eq!(iso(951_782_400.0), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn iso_round_trips() {
        assert_eq!(parse_iso("2026-10-16T13:00:00.000Z"), Some(1_792_155_600.0));
        assert_eq!(parse_iso(&iso(951_782_400.0)), Some(951_782_400.0));
        // The API's "never expires" sentinel must parse without overflowing.
        assert!(parse_iso("+275760-09-13T00:00:00.000Z").is_some_and(|t| t > 8e12));
        assert_eq!(parse_iso("garbage"), None);
    }

    #[test]
    fn the_log_headers_time_reads() {
        assert_eq!(parse_ctime("Tue Nov 10 00:07:35 2026").map(iso).as_deref(), Some("2026-11-10T00:07:35Z"));
        assert_eq!(parse_ctime("Sat Feb  1 09:05:00 2025").map(iso).as_deref(), Some("2025-02-01T09:05:00Z"));
        assert_eq!(parse_ctime("Tue Nov 10 2026"), None);
        assert_eq!(parse_ctime("Tue Foo 10 00:07:35 2026"), None);
    }

    #[test]
    fn resets() {
        // Friday 2026-10-09 15:15 UTC.
        let t = 1_791_558_900.0;
        assert_eq!(iso(next_daily_reset(t)), "2026-10-10T00:00:00Z");
        assert_eq!(iso(next_weekly_reset(t)), "2026-10-12T00:00:00Z");
        // Exactly at a reset, the next one is a day / a week later.
        assert_eq!(iso(next_weekly_reset(1_791_763_200.0)), "2026-10-19T00:00:00Z");
    }
}
