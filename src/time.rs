//! Time and day handling. Port of `Time.java` and the `Day` class, extended with
//! the MTA's official service periods and a "now in New York" clock.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError(pub String);

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for InputError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

/// Port of `Day.DayType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayType {
    Weekday,
    Weekend,
}

impl Weekday {
    pub const ALL: [Weekday; 7] = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ];

    /// Port of `Day.validate`: accepts full names ("saturday") and, as an
    /// extension, three-letter abbreviations ("sat"), any case.
    pub fn parse(s: &str) -> Result<Weekday, InputError> {
        let s = s.trim().to_ascii_lowercase();
        let full = [
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
        ];
        for (i, name) in full.iter().enumerate() {
            if s == *name || (s.len() >= 3 && name.starts_with(s.as_str())) {
                return Ok(Weekday::ALL[i]);
            }
        }
        Err(InputError("Incorrect/invalid input of day.".into()))
    }

    /// Port of `Day.type`.
    pub fn day_type(self) -> DayType {
        match self {
            Weekday::Sat | Weekday::Sun => DayType::Weekend,
            _ => DayType::Weekday,
        }
    }

    pub fn is_weekend(self) -> bool {
        self.day_type() == DayType::Weekend
    }

    pub fn short(self) -> &'static str {
        ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"][self as usize]
    }

    pub fn long(self) -> &'static str {
        [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ][self as usize]
    }

    pub fn next(self) -> Weekday {
        Weekday::ALL[(self as usize + 1) % 7]
    }

    pub fn prev(self) -> Weekday {
        Weekday::ALL[(self as usize + 6) % 7]
    }
}

/// The MTA's named service periods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Rush,
    Midday,
    Evening,
    Weekend,
    LateNight,
}

impl Period {
    pub fn label(self) -> &'static str {
        match self {
            Period::Rush => "rush hours",
            Period::Midday => "middays",
            Period::Evening => "evenings",
            Period::Weekend => "weekends",
            Period::LateNight => "late nights",
        }
    }
}

/// A moment in the week: a day plus minutes since midnight (0..1440).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct When {
    pub day: Weekday,
    pub minutes: u16,
}

pub const LATE_NIGHT_END: u16 = 6 * 60 + 30; // 06:30
const AM_RUSH_END: u16 = 9 * 60 + 30; // 09:30
const PM_RUSH_START: u16 = 15 * 60 + 30; // 15:30
const PM_RUSH_END: u16 = 20 * 60; // 20:00
/// Services marked plain "weekdays" on the map (e.g., the 5 to Flatbush Av, the
/// <6>) run into the early evening. The map doesn't give an exact end time; we
/// use 9:00 PM.
const WEEKDAY_DAYTIME_END: u16 = 21 * 60;

impl When {
    pub fn new(day: Weekday, minutes: u16) -> When {
        When {
            day,
            minutes: minutes % 1440,
        }
    }

    pub fn hhmm(self) -> (u16, u16) {
        (self.minutes / 60, self.minutes % 60)
    }

    pub fn period(self) -> Period {
        let m = self.minutes;

        if m < LATE_NIGHT_END {
            Period::LateNight
        } else if self.day.is_weekend() {
            Period::Weekend
        } else if m < AM_RUSH_END || (PM_RUSH_START..PM_RUSH_END).contains(&m) {
            Period::Rush
        } else if m < PM_RUSH_START {
            Period::Midday
        } else {
            Period::Evening
        }
    }

    pub fn is_am(self) -> bool {
        self.minutes < 12 * 60
    }
    pub fn is_late_night(self) -> bool {
        self.period() == Period::LateNight
    }
    pub fn is_rush(self) -> bool {
        self.period() == Period::Rush
    }
    pub fn is_evening(self) -> bool {
        self.period() == Period::Evening
    }
    pub fn is_weekend_daytime(self) -> bool {
        self.period() == Period::Weekend
    }
    pub fn is_weekday_daytime(self) -> bool {
        !self.day.is_weekend()
            && self.minutes >= LATE_NIGHT_END
            && self.minutes < WEEKDAY_DAYTIME_END
    }
    pub fn is_weekday_not_night(self) -> bool {
        !self.day.is_weekend() && self.minutes >= LATE_NIGHT_END
    }

    pub fn plus_minutes(self, mins: u32) -> When {
        let total = self.minutes as u32 + mins;
        let mut day = self.day;
        for _ in 0..(total / 1440) {
            day = day.next();
        }
        When::new(day, (total % 1440) as u16)
    }

    pub fn clock(self) -> String {
        let (h, m) = self.hhmm();
        format!("{:02}:{:02}", h, m)
    }

    pub fn military(self) -> String {
        let (h, m) = self.hhmm();
        format!("{:02}{:02}", h, m)
    }

    /// The current time in New York City (America/New York, with US DST rules),
    /// computed from the system clock without any timezone database.
    pub fn now_nyc() -> When {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        When::from_unix_nyc(secs)
    }

    pub fn from_unix_nyc(unix_secs: i64) -> When {
        let local = unix_secs + nyc_offset_secs(unix_secs);
        let days = local.div_euclid(86_400);
        let secs_of_day = local.rem_euclid(86_400);
        // 1970-01-01 was a Thursday.
        let wd = Weekday::ALL[((days + 3).rem_euclid(7)) as usize];

        When::new(wd, (secs_of_day / 60) as u16)
    }
}

/// Port of `Time.validate`, returning the parsed minutes after midnight.
/// Accepts military time "930", "0930", "2300", "2400" and, as an extension,
/// "9:30" / "23:00".
pub fn parse_time(input: &str) -> Result<u16, InputError> {
    let err = || InputError("Incorrect/invalid input of time. Use military standard.".into());
    let t: String = input.trim().chars().filter(|c| *c != ':').collect();

    if t.len() < 3 || t.len() > 4 || !t.chars().all(|c| c.is_ascii_digit()) {
        return Err(err());
    }
    let split = t.len() - 2;
    let hrs: u16 = t[..split].parse().map_err(|_| err())?;
    let mins: u16 = t[split..].parse().map_err(|_| err())?;
    
    if (hrs == 24 && mins > 0) || mins > 59 || hrs > 24 {
        return Err(err());
    }
    Ok((hrs % 24) * 60 + mins)
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_year(days: i64) -> i64 {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };

    if m <= 2 {
        y + 1
    } else {
        y
    }
}

/// n-th Sunday of a month, as days since epoch.
fn nth_sunday(year: i64, month: i64, n: i64) -> i64 {
    let first = days_from_civil(year, month, 1);
    let wd = (first + 4).rem_euclid(7); // 0 = Sunday
    let first_sunday = first + (7 - wd) % 7;

    first_sunday + 7 * (n - 1)
}

/// UTC offset of New York at a given instant: EDT (-4h) from the second Sunday
/// of March 2:00 local to the first Sunday of November 2:00 local, else EST (-5h).
fn nyc_offset_secs(unix_secs: i64) -> i64 {
    let year = civil_year(unix_secs.div_euclid(86_400));
    let dst_start = nth_sunday(year, 3, 2) * 86_400 + 7 * 3600;
    let dst_end = nth_sunday(year, 11, 1) * 86_400 + 6 * 3600;

    if unix_secs >= dst_start && unix_secs < dst_end {
        -4 * 3600
    } else {
        -5 * 3600
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn military_time_validation_matches_java() {
        assert_eq!(parse_time("2300"), Ok(23 * 60));
        assert_eq!(parse_time("930"), Ok(9 * 60 + 30));
        assert_eq!(parse_time("0000"), Ok(0));
        assert_eq!(parse_time("2400"), Ok(0));
        assert_eq!(parse_time("9:30"), Ok(570));
        assert!(parse_time("2401").is_err());
        assert!(parse_time("1260").is_err());
        assert!(parse_time("2500").is_err());
        assert!(parse_time("12").is_err());
        assert!(parse_time("12345").is_err());
        assert!(parse_time("ab00").is_err());
    }

    #[test]
    fn day_parsing() {
        assert_eq!(Weekday::parse("Saturday"), Ok(Weekday::Sat));
        assert_eq!(Weekday::parse("tue"), Ok(Weekday::Tue));
        assert!(Weekday::parse("funday").is_err());
        assert_eq!(Weekday::Sun.day_type(), DayType::Weekend);
        assert_eq!(Weekday::Wed.day_type(), DayType::Weekday);
    }

    #[test]
    fn periods() {
        let w = |d, h: u16, m: u16| When::new(d, h * 60 + m);
        assert_eq!(w(Weekday::Mon, 8, 0).period(), Period::Rush);
        assert_eq!(w(Weekday::Mon, 12, 0).period(), Period::Midday);
        assert_eq!(w(Weekday::Mon, 17, 0).period(), Period::Rush);
        assert_eq!(w(Weekday::Mon, 21, 0).period(), Period::Evening);
        assert_eq!(w(Weekday::Mon, 3, 0).period(), Period::LateNight);
        assert_eq!(w(Weekday::Sat, 14, 0).period(), Period::Weekend);
        assert_eq!(w(Weekday::Sat, 2, 0).period(), Period::LateNight);
    }

    #[test]
    fn nyc_clock_handles_dst() {
        // 2026-11-17 21:05 UTC = Tue 16:05 EST
        let t = days_from_civil(2026, 11, 17) * 86_400 + 21 * 3600 + 5 * 60;
        let w = When::from_unix_nyc(t);
        assert_eq!((w.day, w.clock()), (Weekday::Tue, "16:05".to_string()));
        // 2026-07-04 16:00 UTC = Sat 12:00 EDT
        let t = days_from_civil(2026, 7, 4) * 86_400 + 16 * 3600;
        let w = When::from_unix_nyc(t);
        assert_eq!((w.day, w.clock()), (Weekday::Sat, "12:00".to_string()));
    }

    #[test]
    fn plus_minutes_rolls_over() {
        let w = When::new(Weekday::Sun, 23 * 60 + 50).plus_minutes(20);
        assert_eq!((w.day, w.clock()), (Weekday::Mon, "00:10".to_string()));
    }
}
