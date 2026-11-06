// SPDX-License-Identifier: MPL-2.0
//! lib/log/prettyhandler.go: warnings use stdout, ANSI colors and local RFC3339.
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn warn(message: &str) {
    // Explicit reproducible-clock control, shared with document exporters.
    // Ordinary invocations retain Go's local wall clock.
    let injected = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| (0..=253402300799).contains(value));
    let seconds = injected.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    });
    let timestamp = timestamp(seconds, injected.is_some());
    // Embedders may keep process logs separate from the command's API output,
    // just as Go's PrettyHandler bypasses xmain.State.Stdout. The subprocess
    // adapter forwards these bytes to its own process stdout unchanged.
    if let Some(path) = std::env::var_os("D2_RS_CLI_PROCESS_LOG") {
        if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path) {
            let _ = write_warning(&mut file, &timestamp, message);
            return;
        }
    }
    let _ = write_warning(&mut std::io::stdout().lock(), &timestamp, message);
}

fn write_warning(writer: &mut impl Write, timestamp: &str, message: &str) -> std::io::Result<()> {
    writeln!(
        writer,
        "\x1b[2m{timestamp}\x1b[0m \x1b[33mWARN \x1b[0m {message}"
    )
}

// tm_gmtoff is a C long, which is 32 bits on some targets.
#[allow(clippy::unnecessary_cast)]
fn timestamp(seconds: i64, utc: bool) -> String {
    #[cfg(unix)]
    if !utc {
        let seconds = seconds as libc::time_t;
        let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
        // SAFETY: both pointers are valid and localtime_r initializes the
        // output on success. No pointer into libc's shared tm storage escapes.
        if !unsafe { libc::localtime_r(&seconds, local.as_mut_ptr()) }.is_null() {
            let local = unsafe { local.assume_init() };
            return format_timestamp(
                local.tm_year + 1900,
                local.tm_mon + 1,
                local.tm_mday,
                local.tm_hour,
                local.tm_min,
                local.tm_sec,
                local.tm_gmtoff,
            );
        }
    }
    #[cfg(not(unix))]
    let _ = utc;
    // UTC fallback on hosts without a native timezone interface. Gregorian
    // civil date conversion, also used for the explicit UTC clock on Unix.
    let days = seconds.div_euclid(86400) + 719468;
    let era = days.div_euclid(146097);
    let day_of_era = days - era * 146097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let time = seconds.rem_euclid(86400);
    format_timestamp(
        year as i32,
        month as i32,
        day as i32,
        (time / 3600) as i32,
        (time / 60 % 60) as i32,
        (time % 60) as i32,
        0,
    )
}

fn format_timestamp(
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
    second: i32,
    offset: i64,
) -> String {
    let zone = if offset == 0 {
        "Z".to_owned()
    } else {
        let minutes = offset.abs() / 60;
        format!(
            "{}{:02}:{:02}",
            if offset < 0 { '-' } else { '+' },
            minutes / 60,
            minutes % 60
        )
    };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{zone}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn warning_preserves_color_spacing_and_message_bytes() {
        let mut output = Vec::new();
        write_warning(&mut output, "2000-01-02T03:04:05Z", "/a b/uf.d2").unwrap();
        assert_eq!(
            output,
            b"\x1b[2m2000-01-02T03:04:05Z\x1b[0m \x1b[33mWARN \x1b[0m /a b/uf.d2\n"
        );
    }
    #[test]
    fn clock_and_zone_follow_go_rfc3339() {
        assert_eq!(timestamp(946782245, true), "2000-01-02T03:04:05Z");
        assert_eq!(timestamp(951782400, true), "2000-02-29T00:00:00Z");
        assert_eq!(timestamp(253402300799, true), "9999-12-31T23:59:59Z");
        assert_eq!(
            format_timestamp(2026, 10, 25, 11, 0, 0, -25200),
            "2026-10-25T11:00:00-07:00"
        );
        assert_eq!(
            format_timestamp(2026, 10, 25, 11, 0, 0, 20700),
            "2026-10-25T11:00:00+05:45"
        );
    }
}
