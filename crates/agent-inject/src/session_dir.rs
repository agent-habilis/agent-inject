//! Where a session saves when no `<dir>` is given:
//! `$AGENT_INJECT_DIR/<id>/`, or `/tmp/agent-inject/<id>/`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::SystemTime;

/// `/tmp`, not `$TMPDIR`: one path that every shell and every agent sees.
const DEFAULT_BASE: &str = "/tmp/agent-inject";

/// The folder for this session: `explicit` when given, else a fresh id under
/// `AGENT_INJECT_DIR` or [`DEFAULT_BASE`].
pub(crate) fn resolve(explicit: Option<PathBuf>) -> PathBuf {
    pick(
        explicit,
        std::env::var_os("AGENT_INJECT_DIR"),
        &session_id(SystemTime::now(), rand::random()),
    )
}

fn pick(explicit: Option<PathBuf>, base: Option<OsString>, id: &str) -> PathBuf {
    explicit.unwrap_or_else(|| {
        base.filter(|base| !base.is_empty())
            .map_or_else(|| PathBuf::from(DEFAULT_BASE), PathBuf::from)
            .join(id)
    })
}

/// `2026-10-02T19-30-12-a3f9`: UTC, so it sorts by time with no timezone
/// crate, and `suffix` keeps two sessions in the same second apart.
fn session_id(now: SystemTime, suffix: u16) -> String {
    let secs = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = i64::try_from(secs / 86_400).expect("days since 1970 fit in i64");
    let (year, month, day) = civil_from_days(days);
    let of_day = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}-{:02}-{:02}-{suffix:04x}",
        of_day / 3600,
        of_day / 60 % 60,
        of_day % 60,
    )
}

/// Days since 1970-01-01 to `(year, month, day)`, after Howard Hinnant's
/// `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Shifted so the era starts on 0000-03-01, which puts the leap day last.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let doe = shifted.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        u32::try_from(month).expect("month is 1..=12"),
        u32::try_from(day).expect("day is 1..=31"),
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use super::{civil_from_days, pick, session_id};

    #[test]
    fn days_map_to_the_right_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_728), (2026, 10, 2));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn the_id_is_a_utc_timestamp_and_four_hex_digits() {
        // 2026-10-02 19:30:12 UTC.
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(20_728 * 86_400 + 70_212);
        assert_eq!(session_id(now, 0xa3f9), "2026-10-02T19-30-12-a3f9");
        assert_eq!(session_id(now, 0x000b), "2026-10-02T19-30-12-000b");
    }

    #[test]
    fn an_explicit_dir_beats_the_env_and_the_env_beats_the_default() {
        let id = "2026-10-02T19-30-12-a3f9";
        assert_eq!(
            pick(Some(PathBuf::from("/x")), Some("/env".into()), id),
            Path::new("/x")
        );
        assert_eq!(
            pick(None, Some("/env".into()), id),
            Path::new("/env").join(id)
        );
        assert_eq!(
            pick(None, None, id),
            Path::new("/tmp/agent-inject").join(id)
        );
        // An empty variable is unset, not the current directory.
        assert_eq!(
            pick(None, Some("".into()), id),
            Path::new("/tmp/agent-inject").join(id)
        );
    }
}
