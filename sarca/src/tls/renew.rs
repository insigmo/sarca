use chrono::{DateTime, Duration, Utc};

/// Schedule certificate renewal one day before `notAfter` (LE short-lived profile).
pub fn renew_at(not_after: DateTime<Utc>) -> DateTime<Utc> {
    not_after - Duration::days(1)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn renew_at_is_one_day_before_not_after() {
        let not_after = Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
        let renew = renew_at(not_after);
        assert_eq!(renew, Utc.with_ymd_and_hms(2026, 7, 31, 12, 0, 0).unwrap());
    }
}
