//! Candidates that are computed instead of looked up: today's date and time.

const WEEKDAYS: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Moment {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

/// Candidates for a reading that stands for a date or time, such as きょう.
pub fn moment(reading: &str, now: Moment) -> Vec<String> {
    let day = |offset: i64| shift(now, offset);
    match reading {
        "きょう" | "ひづけ" => dates(day(0)),
        "きのう" => dates(day(-1)),
        "おととい" => dates(day(-2)),
        "あした" => dates(day(1)),
        "あさって" => dates(day(2)),
        "いま" | "じこく" => times(now),
        "にちじ" => {
            let mut found = vec![format!(
                "{}/{}/{} {:02}:{:02}",
                now.year, now.month, now.day, now.hour, now.minute
            )];
            found.push(format!(
                "{}年{}月{}日 {}時{}分",
                now.year, now.month, now.day, now.hour, now.minute
            ));
            found
        }
        "ことし" => years(now.year),
        "きょねん" => years(now.year - 1),
        "らいねん" => years(now.year + 1),
        "こんげつ" => months(now.year, now.month as i32),
        "せんげつ" => months(now.year, now.month as i32 - 1),
        "らいげつ" => months(now.year, now.month as i32 + 1),
        _ => Vec::new(),
    }
}

fn dates(date: Moment) -> Vec<String> {
    let weekday = WEEKDAYS[weekday(date.year, date.month, date.day)];
    vec![
        format!("{}/{}/{}", date.year, date.month, date.day),
        format!("{}年{}月{}日", date.year, date.month, date.day),
        format!("{}年{}月{}日({weekday})", date.year, date.month, date.day),
        format!("{}月{}日", date.month, date.day),
        format!("{}/{}", date.month, date.day),
        format!("{}-{:02}-{:02}", date.year, date.month, date.day),
        format!(
            "{}{}月{}日",
            era(date.year, date.month, date.day),
            date.month,
            date.day
        ),
        format!("{weekday}曜日"),
    ]
}

fn times(now: Moment) -> Vec<String> {
    let (noon, hour) = (if now.hour < 12 { "午前" } else { "午後" }, now.hour % 12);
    vec![
        format!("{:02}:{:02}", now.hour, now.minute),
        format!("{}時{}分", now.hour, now.minute),
        format!("{noon}{hour}時{}分", now.minute),
    ]
}

fn years(year: i32) -> Vec<String> {
    vec![format!("{year}年"), era(year, 12, 31), year.to_string()]
}

fn months(year: i32, month: i32) -> Vec<String> {
    let (year, month) = (
        year + (month - 1).div_euclid(12),
        (month - 1).rem_euclid(12) + 1,
    );
    vec![format!("{month}月"), format!("{year}年{month}月")]
}

/// The Japanese era year, such as 令和8年 or 令和元年. Years before 令和 are
/// left as they are.
fn era(year: i32, month: u32, day: u32) -> String {
    if (year, month, day) < (2019, 5, 1) {
        return format!("{year}年");
    }
    match year - 2018 {
        1 => "令和元年".to_owned(),
        years => format!("令和{years}年"),
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

/// 0 is Sunday.
fn weekday(year: i32, month: u32, day: u32) -> usize {
    (days_from_civil(year, month, day) + 4).rem_euclid(7) as usize
}

fn shift(now: Moment, offset: i64) -> Moment {
    let (year, month, day) =
        civil_from_days(days_from_civil(now.year, now.month, now.day) + offset);
    Moment {
        year,
        month,
        day,
        ..now
    }
}

/// The current local date and time.
#[cfg(windows)]
pub fn now() -> Option<Moment> {
    let mut time = std::mem::MaybeUninit::<windows_sys::Win32::Foundation::SYSTEMTIME>::uninit();
    // SAFETY: GetLocalTime fills the whole structure it is given.
    let time = unsafe {
        windows_sys::Win32::System::SystemInformation::GetLocalTime(time.as_mut_ptr());
        time.assume_init()
    };
    Some(Moment {
        year: i32::from(time.wYear),
        month: u32::from(time.wMonth),
        day: u32::from(time.wDay),
        hour: u32::from(time.wHour),
        minute: u32::from(time.wMinute),
    })
}

/// The current date and time in UTC, where the local time zone is not known.
#[cfg(not(windows))]
pub fn now() -> Option<Moment> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let in_day = seconds.rem_euclid(86_400);
    Some(Moment {
        year,
        month,
        day,
        hour: (in_day / 3600) as u32,
        minute: (in_day % 3600 / 60) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Moment = Moment {
        year: 2026,
        month: 11,
        day: 25,
        hour: 1,
        minute: 24,
    };

    #[test]
    fn writes_dates_with_the_weekday_and_era() {
        let today = moment("きょう", NOW);
        assert_eq!(today[0], "2026/11/25");
        assert_eq!(today[1], "2026年11月25日");
        assert_eq!(today[2], "2026年11月25日(水)");
        assert_eq!(today[6], "令和8年11月25日");
        assert_eq!(today[7], "水曜日");
        assert_eq!(moment("あした", NOW)[0], "2026/11/26");
        assert_eq!(moment("あさって", NOW)[0], "2026/11/27");
        assert_eq!(moment("きのう", NOW)[0], "2026/11/24");
        assert_eq!(moment("おととい", NOW)[0], "2026/11/23");
        assert!(moment("ねこ", NOW).is_empty());
    }

    #[test]
    fn dates_cross_month_year_and_leap_boundaries() {
        let end = Moment {
            year: 2026,
            month: 12,
            day: 31,
            ..NOW
        };
        assert_eq!(moment("あした", end)[0], "2027/1/1");
        let leap = Moment {
            year: 2028,
            month: 2,
            day: 28,
            ..NOW
        };
        assert_eq!(moment("あした", leap)[0], "2028/2/29");
        assert_eq!(moment("あさって", leap)[0], "2028/3/1");
        let start = Moment {
            year: 2026,
            month: 3,
            day: 1,
            ..NOW
        };
        assert_eq!(moment("きのう", start)[0], "2026/2/28");
        assert_eq!(weekday(1970, 1, 1), 4);
        assert_eq!(weekday(2000, 2, 29), 2);
    }

    #[test]
    fn writes_times_months_and_years() {
        assert_eq!(moment("いま", NOW), ["01:24", "1時24分", "午前1時24分"]);
        let afternoon = Moment { hour: 15, ..NOW };
        assert_eq!(moment("いま", afternoon)[2], "午後3時24分");
        assert_eq!(moment("にちじ", NOW)[0], "2026/11/25 01:24");
        assert_eq!(moment("ことし", NOW), ["2026年", "令和8年", "2026"]);
        assert_eq!(moment("らいねん", NOW)[1], "令和9年");
        assert_eq!(moment("きょねん", NOW)[0], "2025年");
        assert_eq!(moment("こんげつ", NOW), ["11月", "2026年11月"]);
        assert_eq!(moment("らいげつ", NOW)[1], "2026年12月");
        let december = Moment { month: 12, ..NOW };
        assert_eq!(moment("らいげつ", december), ["1月", "2027年1月"]);
        let january = Moment { month: 1, ..NOW };
        assert_eq!(moment("せんげつ", january), ["12月", "2025年12月"]);
        assert_eq!(era(2019, 5, 1), "令和元年");
        assert_eq!(era(2019, 4, 30), "2019年");
        assert!(now().is_some());
    }
}
