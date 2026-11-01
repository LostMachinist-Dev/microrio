//! One time zone for the whole station.
//!
//! The wire never carries a zone: the node's RTC holds unix seconds and the
//! sync button sends unix seconds, which is unambiguous and needs no tz
//! database on an AVR. Everything a person reads — the clock panel, a chart's
//! time axis, the timestamp column in an exported CSV — is rendered through
//! this one zone instead, so there is a single answer to "what time is that".

use chrono::{DateTime, Local, TimeZone};
use chrono_tz::Tz;

/// The station's display zone.
///
/// `system` is resolved to a real IANA zone where the machine will say what
/// it is, because `chrono`'s own local zone cannot produce an abbreviation —
/// it formats as `-07:00`, where the named zone formats as `PDT`. Telling an
/// operator the offset instead of the name is the sort of small confusion
/// this whole setting exists to remove.
#[derive(Debug, Clone)]
pub struct Zone {
    /// True when this follows the machine rather than being pinned.
    system: bool,
    /// The resolved zone. `None` only when `system` is set and the machine
    /// would not say which zone it is in, in which case we fall back to
    /// chrono's local time and show a numeric offset.
    tz: Option<Tz>,
}

impl Zone {
    /// Resolve a profile string. An unrecognised name falls back to the
    /// system zone rather than failing — a bad profile should not stop the
    /// station opening.
    pub fn parse(s: &str) -> Self {
        if s.trim().is_empty() || s.eq_ignore_ascii_case("system") {
            return Self::system();
        }
        match s.parse::<Tz>() {
            Ok(tz) => Self {
                system: false,
                tz: Some(tz),
            },
            Err(_) => Self::system(),
        }
    }

    pub fn system() -> Self {
        Self {
            system: true,
            tz: iana_time_zone::get_timezone()
                .ok()
                .and_then(|n| n.parse::<Tz>().ok()),
        }
    }

    /// True when `parse` would keep the string rather than falling back.
    pub fn is_known(s: &str) -> bool {
        s.trim().is_empty() || s.eq_ignore_ascii_case("system") || s.parse::<Tz>().is_ok()
    }

    pub fn at(&self, unix_secs: i64) -> Option<DateTime<Tz>> {
        self.tz
            .and_then(|tz| tz.timestamp_opt(unix_secs, 0).single())
    }

    /// `2026-10-25 19:34:07`, or a dash for a timestamp that does not exist in
    /// this zone — the hour a spring-forward skips.
    pub fn fmt(&self, unix_secs: i64, fmt: &str) -> String {
        match self.at(unix_secs) {
            Some(d) => d.format(fmt).to_string(),
            None => match Local.timestamp_opt(unix_secs, 0).single() {
                Some(d) => d.format(fmt).to_string(),
                None => "—".into(),
            },
        }
    }

    pub fn stamp(&self, unix_secs: i64) -> String {
        self.fmt(unix_secs, "%Y-%m-%d %H:%M:%S")
    }

    /// Microsecond resolution, for log rows where frames are 20 ms apart.
    pub fn stamp_us(&self, unix_us: i64) -> String {
        let secs = unix_us.div_euclid(1_000_000);
        let frac = unix_us.rem_euclid(1_000_000);
        format!("{}.{:06}", self.fmt(secs, "%Y-%m-%d %H:%M:%S"), frac)
    }

    pub fn now_stamp(&self) -> String {
        self.stamp(chrono::Utc::now().timestamp())
    }

    /// A filename-safe stamp of now, for suggesting an export name.
    pub fn file_stamp(&self) -> String {
        self.fmt(chrono::Utc::now().timestamp(), "%Y%m%d-%H%M%S")
    }

    /// What to print next to a time so it is never ambiguous: `PDT`, `AEST`,
    /// or a numeric offset when the machine would not name its zone.
    pub fn abbrev(&self) -> String {
        let now = chrono::Utc::now().timestamp();
        match self.at(now) {
            Some(d) => d.format("%Z").to_string(),
            None => Local
                .timestamp_opt(now, 0)
                .single()
                .map(|d| d.format("%:z").to_string())
                .unwrap_or_default(),
        }
    }

    /// Fractional unix seconds as a time of day with milliseconds, for a
    /// chart readout where samples are 20 ms apart.
    pub fn fmt_ms(&self, unix: f64) -> String {
        let secs = unix.floor() as i64;
        let ms = ((unix - unix.floor()) * 1000.0) as u32;
        format!("{}.{ms:03}", self.fmt(secs, "%H:%M:%S"))
    }

    /// UTC offset in seconds at an instant.
    fn offset_s(&self, unix_secs: i64) -> i32 {
        use chrono::Offset;
        match self.at(unix_secs) {
            Some(d) => d.offset().fix().local_minus_utc(),
            None => Local
                .timestamp_opt(unix_secs, 0)
                .single()
                .map(|d| d.offset().local_minus_utc())
                .unwrap_or(0),
        }
    }

    /// `UTC-07:00 · DST not in effect`, or `· no DST` for a zone that never
    /// changes. Daylight saving is whichever of January and July is ahead,
    /// which holds in both hemispheres.
    pub fn offset_summary(&self) -> String {
        use chrono::Datelike;
        let now = chrono::Utc::now();
        let year = now.year();
        let at = |m: u32| {
            chrono::Utc
                .with_ymd_and_hms(year, m, 1, 12, 0, 0)
                .single()
                .map(|d| self.offset_s(d.timestamp()))
                .unwrap_or(0)
        };
        let (jan, jul) = (at(1), at(7));
        let cur = self.offset_s(now.timestamp());
        let sign = if cur < 0 { '-' } else { '+' };
        let off = format!(
            "UTC{sign}{:02}:{:02}",
            cur.abs() / 3600,
            (cur.abs() % 3600) / 60
        );
        let dst = if jan == jul {
            "no DST"
        } else if cur == jan.max(jul) {
            "DST in effect"
        } else {
            "DST not in effect"
        };
        format!("{off} · {dst}")
    }

    /// The name as it belongs on screen: `system` alone says nothing, so it
    /// carries what it resolved to.
    pub fn describe(&self) -> String {
        match (self.system, self.tz) {
            (true, Some(tz)) => format!("system ({})", tz.name()),
            (true, None) => "system (machine will not say which)".into(),
            (false, Some(tz)) => tz.name().to_string(),
            (false, None) => "system".into(),
        }
    }
}

/// A short list for the dropdown. Typing any IANA name still works; this is
/// only so the common cases need no typing.
pub const COMMON_ZONES: &[&str] = &[
    "system",
    "America/Los_Angeles",
    "America/Denver",
    "America/Chicago",
    "America/New_York",
    "UTC",
    "Europe/London",
    "Europe/Berlin",
    "Australia/Sydney",
];
