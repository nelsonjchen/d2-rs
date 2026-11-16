// SPDX-License-Identifier: MPL-2.0
//! lib/log/prettyhandler.go: warnings use stdout, ANSI colors and local RFC3339.
//! util-go cmdlog: `debug:` lines go to stderr while `$DEBUG` is true.
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// d2cli.Run's resolved --debug flag (with `$DEBUG` as its default). When it
/// is true Go levels the slog context at Debug and sets `$DEBUG=1`.
static DEBUG: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_debug(enabled: bool) {
    DEBUG.store(enabled, Ordering::Relaxed);
}

/// Whether d2cli.Run leveled its slog context at Debug (main.go:224-227).
pub(crate) fn debug_enabled() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

/// Whether a cmdlog Debug write would print (no error report for `$DEBUG`).
pub(crate) fn command_debug_active() -> bool {
    debug_enabled()
        || matches!(
            std::env::var_os("DEBUG")
                .unwrap_or_default()
                .as_encoded_bytes(),
            b"1" | b"true"
        )
}

/// util-go cmdlog `ms.Log.Debug.Printf`: debugWriter consults xos.Env.Debug
/// on every write, so `$DEBUG=1` prints these lines even with --debug=false,
/// and an invalid value writes Env.Debug's unterminated error instead.
pub(crate) fn command_debug(message: &str) {
    if !debug_enabled() {
        let value = std::env::var_os("DEBUG").unwrap_or_default();
        match value.as_encoded_bytes() {
            b"" | b"0" | b"false" => return,
            b"1" | b"true" => {}
            bytes => {
                // xos.Env.Bool: fmt.Errorf("$%s must be 0, 1, false or true
                // but got %q"), written by Env.Debug without a newline.
                let _ = write!(
                    std::io::stderr().lock(),
                    "env: $DEBUG must be 0, 1, false or true but got {}",
                    d2_rs_core::quote_go_bytes(bytes)
                );
                return;
            }
        }
    }
    // cmdlog.prefixWriter: "debug:" plus a space before each nonempty line.
    let mut text = String::new();
    for line in message.split_terminator('\n') {
        if line.is_empty() {
            text.push_str("debug:\n");
        } else {
            text.push_str("debug: ");
            text.push_str(line);
            text.push('\n');
        }
    }
    let _ = std::io::stderr().lock().write_all(text.as_bytes());
}

/// lib/log.Debug through the leveled PrettyHandler (stdout, cyan level).
pub(crate) fn debug(message: &str) {
    if debug_enabled() {
        pretty("\x1b[36mDEBUG\x1b[0m", message);
    }
}

pub(crate) fn warn(message: &str) {
    pretty("\x1b[33mWARN \x1b[0m", message);
}

fn pretty(level: &str, message: &str) {
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
            let _ = write_record(&mut file, &timestamp, level, message);
            return;
        }
    }
    let _ = write_record(&mut std::io::stdout().lock(), &timestamp, level, message);
}

#[cfg(test)]
fn write_warning(writer: &mut impl Write, timestamp: &str, message: &str) -> std::io::Result<()> {
    write_record(writer, timestamp, "\x1b[33mWARN \x1b[0m", message)
}

// PrettyHandler.Handle: fmt.Println("<faded ts> <colored %-5s level> msg").
fn write_record(
    writer: &mut impl Write,
    timestamp: &str,
    level: &str,
    message: &str,
) -> std::io::Result<()> {
    writeln!(writer, "\x1b[2m{timestamp}\x1b[0m {level} {message}")
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
