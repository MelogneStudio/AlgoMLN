use chrono::NaiveDate;

/// NSE trading-day calendar. Lists the days the cash equity market is closed
/// (whole-day holidays only). The list is a static constant — the official NSE
/// holiday calendar changes annually, so this module is the single place to
/// update each year.
///
/// **Known limitation (Phase 7):** Muhurat sessions are ignored. The market
/// is treated as closed on the Diwali evening when NSE runs a one-hour
/// special session outside the standard 09:15–15:30 window. Users cannot run
/// a live session during Muhurat trading in Phase 7. Phase 8 can add a
/// dedicated window.
pub struct NseHolidayCalendar {
    holidays: Vec<NaiveDate>,
}

impl NseHolidayCalendar {
    /// Build an empty calendar. Production callers should populate
    /// `holidays` from the latest NSE circular; tests build a calendar with
    /// a single known holiday to verify the predicate.
    pub fn new() -> Self {
        Self {
            holidays: Vec::new(),
        }
    }

    /// Build a calendar with the supplied list. Used by tests and any future
    /// loader that fetches the calendar from an external source.
    pub fn with_holidays(holidays: Vec<NaiveDate>) -> Self {
        Self { holidays }
    }

    /// True if the given date is an NSE trading holiday (whole-day closure).
    pub fn is_holiday(&self, d: NaiveDate) -> bool {
        self.holidays.iter().any(|h| *h == d)
    }

    /// Total number of holidays currently in the calendar. Mostly for tests
    /// and startup logging.
    pub fn len(&self) -> usize {
        self.holidays.len()
    }

    pub fn is_empty(&self) -> bool {
        self.holidays.is_empty()
    }
}

impl Default for NseHolidayCalendar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_calendar_returns_false() {
        let cal = NseHolidayCalendar::new();
        let any_day = NaiveDate::from_ymd_opt(2026, 1, 26).unwrap();
        assert!(!cal.is_holiday(any_day));
    }

    #[test]
    fn configured_holiday_matches() {
        let holidays = vec![NaiveDate::from_ymd_opt(2026, 1, 26).unwrap()];
        let cal = NseHolidayCalendar::with_holidays(holidays);
        assert!(cal.is_holiday(NaiveDate::from_ymd_opt(2026, 1, 26).unwrap()));
        assert!(!cal.is_holiday(NaiveDate::from_ymd_opt(2026, 1, 27).unwrap()));
    }
}