//! Times for `mumble history`'s filters: local ISO stamps, and `--since` /
//! `--until` values such as `2026-10-15`, `today`, or `7d`.

use color_eyre::eyre::{Result, eyre};

#[cfg(unix)]
use libc::mktime;

// The Universal CRT exports `mktime` only as its 64-bit form.
#[cfg(windows)]
unsafe extern "C" {
    #[link_name = "_mktime64"]
    fn mktime(tm: *mut libc::tm) -> libc::time_t;
}

fn local_tm(milliseconds: u64) -> libc::tm {
    let seconds = (milliseconds / 1_000) as libc::time_t;
    // SAFETY: `localtime_r` and `localtime_s` write only into the provided
    // zeroed `tm`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    #[cfg(unix)]
    unsafe {
        libc::localtime_r(&seconds, &mut tm)
    };
    #[cfg(windows)]
    unsafe {
        libc::localtime_s(&mut tm, &seconds)
    };
    tm
}

/// Local time with its UTC offset, such as `2026-10-15T14:58:31-07:00`.
pub fn iso_local(milliseconds: u64) -> String {
    let tm = local_tm(milliseconds);
    #[cfg(unix)]
    let offset = tm.tm_gmtoff;
    // The local wall clock read as UTC, less the instant itself.
    #[cfg(windows)]
    let offset = {
        let wall = days_from_civil(
            i64::from(tm.tm_year) + 1_900,
            i64::from(tm.tm_mon) + 1,
            i64::from(tm.tm_mday),
        ) * 86_400
            + i64::from(tm.tm_hour) * 3_600
            + i64::from(tm.tm_min) * 60
            + i64::from(tm.tm_sec);
        wall - (milliseconds / 1_000) as i64
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
        tm.tm_year + 1_900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        if offset < 0 { '-' } else { '+' },
        offset.abs() / 3_600,
        offset.abs() % 3_600 / 60
    )
}

fn local_to_ms(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<u64> {
    // SAFETY: `mktime` reads and normalizes only the provided `tm`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = year - 1_900;
    tm.tm_mon = month as i32 - 1;
    tm.tm_mday = day as i32;
    tm.tm_hour = hour as i32;
    tm.tm_min = minute as i32;
    tm.tm_sec = second as i32;
    tm.tm_isdst = -1;
    let seconds = unsafe { mktime(&mut tm) };
    u64::try_from(seconds).ok().map(|seconds| seconds * 1_000)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// `2026-10-15`, `2026-10-15T14:00`, RFC 3339 with `Z` or an offset,
/// `today`, `yesterday`, `now`, relative `90m`/`36h`/`7d`/`2w`, or epoch ms.
/// A date alone means local midnight; as an upper bound it covers the day.
pub fn parse_time(input: &str, now_ms: u64, end_of_day: bool) -> Result<u64> {
    let text = input.trim().to_ascii_lowercase();
    let invalid = || {
        eyre!(
            "Could not read the time {input:?}; use 2026-10-15, 2026-10-15T14:00, today, yesterday, or 7d."
        )
    };
    let today = local_tm(now_ms);
    let midnight = |days_ago: i32, end: bool| {
        // SAFETY: `mktime` reads and normalizes only the provided `tm`,
        // including out-of-range days across months.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = today.tm_year;
        tm.tm_mon = today.tm_mon;
        tm.tm_mday = today.tm_mday - days_ago + i32::from(end);
        tm.tm_isdst = -1;
        u64::try_from(unsafe { mktime(&mut tm) })
            .ok()
            .map(|seconds| seconds * 1_000)
    };
    match text.as_str() {
        "now" => return Ok(now_ms),
        "today" => return midnight(0, end_of_day).ok_or_else(invalid),
        "yesterday" => return midnight(1, end_of_day).ok_or_else(invalid),
        _ => {}
    }
    if text.bytes().all(|byte| byte.is_ascii_digit()) && text.len() >= 10 {
        let value: u64 = text.parse().map_err(|_| invalid())?;
        return Ok(if text.len() <= 10 {
            value * 1_000
        } else {
            value
        });
    }
    let relative = text.strip_suffix(" ago").unwrap_or(&text).trim();
    if let Some(split) = relative.find(|character: char| !character.is_ascii_digit())
        && split > 0
    {
        let (amount, unit) = relative.split_at(split);
        let unit_ms = match unit.trim() {
            "m" | "min" | "mins" | "minutes" => Some(60_000),
            "h" | "hr" | "hrs" | "hours" => Some(3_600_000),
            "d" | "day" | "days" => Some(86_400_000),
            "w" | "week" | "weeks" => Some(7 * 86_400_000),
            _ => None,
        };
        if let Some(unit_ms) = unit_ms {
            let amount: u64 = amount.parse().map_err(|_| invalid())?;
            return Ok(now_ms.saturating_sub(amount.checked_mul(unit_ms).ok_or_else(invalid)?));
        }
    }
    let pattern = regex::Regex::new(
        r"^(\d{4})-(\d{2})-(\d{2})(?:[t ](\d{2}):(\d{2})(?::(\d{2})(?:\.(?P<fraction>\d+))?)?)?\s*(?P<zone>z|[+-]\d{2}:?\d{2})?$",
    )
    .expect("valid time pattern");
    let captures = pattern.captures(&text).ok_or_else(invalid)?;
    let number = |index: usize| {
        captures
            .get(index)
            .map_or(Ok(0), |value| value.as_str().parse::<u32>())
            .map_err(|_| invalid())
    };
    let (year, month, day) = (number(1)?, number(2)?, number(3)?);
    let has_time = captures.get(4).is_some();
    let (hour, minute, second) = (number(4)?, number(5)?, number(6)?);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    if day == 0 || day > days || hour > 23 || minute > 59 || second > 59 {
        return Err(invalid());
    }
    let day = if !has_time && end_of_day {
        day + 1
    } else {
        day
    };
    let milliseconds = match captures.name("zone").map(|zone| zone.as_str()) {
        Some(zone) => {
            let offset_seconds = if zone == "z" {
                0
            } else {
                let sign = if zone.starts_with('-') { -1 } else { 1 };
                let digits = zone[1..].replace(':', "");
                let hours: i64 = digits[..2].parse().map_err(|_| invalid())?;
                let minutes: i64 = digits[2..].parse().map_err(|_| invalid())?;
                if hours > 23 || minutes > 59 {
                    return Err(invalid());
                }
                sign * (hours * 3_600 + minutes * 60)
            };
            let seconds = days_from_civil(year.into(), month.into(), day.into()) * 86_400
                + i64::from(hour) * 3_600
                + i64::from(minute) * 60
                + i64::from(second)
                - offset_seconds;
            u64::try_from(seconds)
                .map(|seconds| seconds * 1_000)
                .map_err(|_| invalid())
        }
        None => local_to_ms(year as i32, month, day, hour, minute, second).ok_or_else(invalid),
    }?;
    let fraction = captures.name("fraction").map_or(0, |digits| {
        digits
            .as_str()
            .bytes()
            .take(3)
            .zip([100, 10, 1])
            .map(|(digit, scale)| u64::from(digit - b'0') * scale)
            .sum::<u64>()
    });
    Ok(milliseconds + fraction)
}

pub struct TimeRange {
    since: Option<u64>,
    until: Option<u64>,
}

impl TimeRange {
    pub fn parse(since: Option<&str>, until: Option<&str>, now_ms: u64) -> Result<Self> {
        let since = since
            .filter(|value| !value.trim().is_empty())
            .map(|value| parse_time(value, now_ms, false))
            .transpose()?;
        let until = until
            .filter(|value| !value.trim().is_empty())
            .map(|value| parse_time(value, now_ms, true))
            .transpose()?;
        if since
            .zip(until)
            .is_some_and(|(since, until)| since >= until)
        {
            return Err(eyre!("since must be earlier than until"));
        }
        Ok(Self { since, until })
    }

    pub fn contains(&self, timestamp_ms: u64) -> bool {
        self.since.is_none_or(|since| timestamp_ms >= since)
            && self.until.is_none_or(|until| timestamp_ms < until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_790_100_000_000;

    #[test]
    fn parses_absolute_relative_and_named_times() {
        assert_eq!(
            parse_time("2026-10-15T21:58:31Z", NOW, false).unwrap(),
            1_792_101_511_000
        );
        assert_eq!(
            parse_time("2026-10-15T14:58:31-07:00", NOW, false).unwrap(),
            1_792_101_511_000
        );
        assert_eq!(
            parse_time("1792101511335", NOW, false).unwrap(),
            1_792_101_511_335
        );
        assert_eq!(parse_time("7d", NOW, false).unwrap(), NOW - 7 * 86_400_000);
        let day = parse_time("2026-10-15", NOW, false).unwrap();
        assert_eq!(
            parse_time("2026-10-15", NOW, true).unwrap() - day,
            86_400_000
        );
        assert_eq!(
            parse_time("yesterday", NOW, true).unwrap(),
            parse_time("today", NOW, false).unwrap()
        );
        assert!(parse_time("last tuesday", NOW, false).is_err());
    }
}
