//! RFC 3339 timestamp formatting without pulling in `chrono`.
//!
//! Chrono is small but not free in a `opt-level = "z"` WASM build;
//! one strftime is not worth it. The civil_from_days algorithm here is
//! Howard Hinnant's branchless POSIX-day-to-Gregorian-date conversion,
//! the canonical reference implementation.

/// Render an RFC 3339 timestamp for the current host wallclock.
///
/// On wasm32 this calls the SDK `time::now()` host fn; on every other
/// target (host-target unit tests) it reads `std::time::SystemTime`
/// directly. Either source's failure mode falls back to the Unix
/// epoch — records stay structurally valid with an obviously-fake
/// timestamp the operator can grep for.
#[must_use]
pub(crate) fn now_rfc3339() -> String {
    let millis = current_unix_millis();
    let secs = (millis / 1000) as i64;
    let sub_ms = (millis % 1000) as u32;
    format_rfc3339(secs, sub_ms)
}

#[cfg(target_family = "wasm")]
fn current_unix_millis() -> u128 {
    match astrid_sdk::prelude::time::now() {
        Ok(t) => t
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis()),
        Err(_) => 0,
    }
}

#[cfg(not(target_family = "wasm"))]
fn current_unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

/// Format a POSIX timestamp as RFC 3339 (`YYYY-MM-DDTHH:MM:SS.mmmZ`).
/// Standalone so tests can drive it deterministically.
#[must_use]
pub(crate) fn format_rfc3339(unix_secs: i64, sub_ms: u32) -> String {
    let total_days = unix_secs.div_euclid(86_400);
    let rem_secs = unix_secs.rem_euclid(86_400) as u32;
    let hour = rem_secs / 3_600;
    let minute = (rem_secs % 3_600) / 60;
    let second = rem_secs % 60;
    let (year, month, day) = days_to_ymd(total_days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{sub_ms:03}Z")
}

/// Convert a day index relative to 1970-01-01 into `(year, month, day)`.
fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year_adj = if m <= 2 { y + 1 } else { y };
    (year_adj as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_epoch() {
        assert_eq!(format_rfc3339(0, 0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn rfc3339_known_date() {
        // 2026-01-15T12:34:56.789Z
        // = 20468 days × 86400 + 12h 34m 56s + 789ms
        // = 1_768_480_496 seconds since the Unix epoch.
        assert_eq!(
            format_rfc3339(1_768_480_496, 789),
            "2026-01-15T12:34:56.789Z"
        );
    }

    #[test]
    fn rfc3339_y2038_safe() {
        assert_eq!(format_rfc3339(2_147_483_647, 0), "2038-01-19T03:14:07.000Z");
    }
}
