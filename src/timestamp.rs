//! The time of assembly, for the `_year`, `_month` etc. constants.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    pub year: i32,
    /// 1 = January.
    pub month: i32,
    pub day: i32,
    /// 0 = Sunday.
    pub weekday: i32,
    pub hours: i32,
    pub minutes: i32,
    pub seconds: i32,
}

impl Timestamp {
    /// Converts seconds since 1970 to a calendar date, without regard to
    /// time zones.
    pub fn from_unix(secs: i64) -> Timestamp {
        let days = secs.div_euclid(86400);
        let rem = secs.rem_euclid(86400);
        // Civil from days, after Howard Hinnant's algorithm.
        let z = days + 719468;
        let era = z.div_euclid(146097);
        let doe = z.rem_euclid(146097);
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
        Timestamp {
            year: year as i32,
            month: month as i32,
            day: day as i32,
            weekday: (days + 4).rem_euclid(7) as i32,
            hours: (rem / 3600) as i32,
            minutes: (rem / 60 % 60) as i32,
            seconds: (rem % 60) as i32,
        }
    }

    /// The current time in UTC. `SOURCE_DATE_EPOCH` overrides the clock,
    /// for reproducible builds.
    pub fn now() -> Timestamp {
        Timestamp::from_unix(Timestamp::seconds())
    }

    /// The current time in local time, if there is a way of finding it on
    /// this system. `SOURCE_DATE_EPOCH` overrides the clock and is taken
    /// as it is, without regard to the time zone.
    pub fn now_local() -> Option<Timestamp> {
        match Timestamp::source_date_epoch() {
            Some(t) => Some(Timestamp::from_unix(t)),
            None => local::local_time(Timestamp::seconds()),
        }
    }

    fn source_date_epoch() -> Option<i64> {
        std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|s| s.trim().parse().ok())
    }

    /// Seconds since 1970, by the clock or as set by `SOURCE_DATE_EPOCH`.
    fn seconds() -> i64 {
        Timestamp::source_date_epoch().unwrap_or_else(|| {
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
        })
    }
}

// The standard library has no way of finding local time, so this is left
// to the C library, on the systems where that has been tested.
#[cfg(target_os = "linux")]
mod local {
    use super::Timestamp;
    use std::os::raw::{c_int, c_long};

    // `struct tm` of the C library. The trailing member is there to make
    // sure of room for what the library writes.
    #[repr(C)]
    struct Tm {
        tm_sec: c_int,
        tm_min: c_int,
        tm_hour: c_int,
        tm_mday: c_int,
        tm_mon: c_int,
        tm_year: c_int,
        tm_wday: c_int,
        tm_yday: c_int,
        tm_isdst: c_int,
        tm_gmtoff: c_long,
        tm_zone: *const u8,
        reserved: [u64; 4],
    }

    extern "C" {
        fn localtime_r(time: *const c_long, tm: *mut Tm) -> *mut Tm;
    }

    pub fn local_time(secs: i64) -> Option<Timestamp> {
        let t = c_long::try_from(secs).ok()?;
        let mut tm = std::mem::MaybeUninit::<Tm>::zeroed();
        // SAFETY: both pointers are valid for the duration of the call and
        // `Tm` is at least as large as the C library's `struct tm`.
        let tm = unsafe {
            if localtime_r(&t, tm.as_mut_ptr()).is_null() {
                return None;
            }
            tm.assume_init()
        };
        Some(Timestamp {
            year: tm.tm_year + 1900,
            month: tm.tm_mon + 1,
            day: tm.tm_mday,
            weekday: tm.tm_wday,
            hours: tm.tm_hour,
            minutes: tm.tm_min,
            seconds: tm.tm_sec,
        })
    }
}

#[cfg(not(target_os = "linux"))]
mod local {
    use super::Timestamp;

    pub fn local_time(_secs: i64) -> Option<Timestamp> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_unix_time() {
        let t = Timestamp::from_unix(0);
        assert_eq!((t.year, t.month, t.day, t.weekday), (1970, 1, 1, 4));
        // 2026-09-27 15:26:40 UTC, a Sunday
        let t = Timestamp::from_unix(1790522800);
        assert_eq!((t.year, t.month, t.day), (2026, 9, 27));
        assert_eq!((t.hours, t.minutes, t.seconds), (15, 26, 40));
        assert_eq!(t.weekday, 0);
        // Leap day
        let t = Timestamp::from_unix(951782400);
        assert_eq!((t.year, t.month, t.day), (2000, 2, 29));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn local_time_follows_the_time_zone() {
        // No time zone is more than a day from UTC.
        let secs = 1790522800;
        let local = local::local_time(secs).unwrap();
        let utc = Timestamp::from_unix(secs);
        assert_eq!(local.minutes % 15, utc.minutes % 15);
        assert_eq!(local.seconds, utc.seconds);
        assert!((local.day - utc.day).abs() <= 1 || local.month != utc.month);
    }

    #[test]
    fn time_is_plausible() {
        let t = Timestamp::now();
        assert!(t.year >= 2024);
        assert!((1..=12).contains(&t.month));
        assert!((1..=31).contains(&t.day));
        assert!((0..=6).contains(&t.weekday));
    }
}
