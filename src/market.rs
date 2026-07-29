use std::sync::Arc;

use chrono::{DateTime, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

/// Trading-session metadata shared by a provider, cache, and chart timeline.
///
/// A market context can contain instruments from several listing exchanges
/// when those exchanges share one calendar and symbol namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketContext {
    pub id: Arc<str>,
    pub symbol_namespace: Arc<str>,
    pub currency: Arc<str>,
    pub timezone: Tz,
    pub regular_open: NaiveTime,
    pub regular_close: NaiveTime,
}

impl MarketContext {
    #[must_use]
    pub fn us_equities() -> Self {
        Self {
            id: Arc::from("us-equities"),
            symbol_namespace: Arc::from("us-equity"),
            currency: Arc::from("USD"),
            timezone: chrono_tz::America::New_York,
            regular_open: NaiveTime::from_hms_opt(9, 30, 0).expect("valid regular-session open"),
            regular_close: NaiveTime::from_hms_opt(16, 0, 0).expect("valid regular-session close"),
        }
    }

    #[must_use]
    pub fn session_date(&self, timestamp: DateTime<Utc>) -> NaiveDate {
        timestamp.with_timezone(&self.timezone).date_naive()
    }

    #[must_use]
    pub fn session_bounds(&self, date: NaiveDate) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let open = resolve_local_time(self.timezone, date, self.regular_open)?;
        let close = resolve_local_time(self.timezone, date, self.regular_close)?;
        (close > open).then_some((open, close))
    }
}

impl Default for MarketContext {
    fn default() -> Self {
        Self::us_equities()
    }
}

/// Opaque provider dataset identity plus the market semantics of its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheIdentity {
    pub namespace: Arc<str>,
    pub market: MarketContext,
}

impl CacheIdentity {
    #[must_use]
    pub fn new(namespace: impl Into<Arc<str>>, market: MarketContext) -> Self {
        Self {
            namespace: namespace.into(),
            market,
        }
    }
}

fn resolve_local_time(timezone: Tz, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    let local = date.and_time(time);
    match timezone.from_local_datetime(&local) {
        LocalResult::Single(timestamp) => Some(timestamp.with_timezone(&Utc)),
        LocalResult::Ambiguous(earlier, later) => Some(earlier.min(later).with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn us_sessions_follow_new_york_daylight_saving_time() {
        let market = MarketContext::us_equities();
        let winter = NaiveDate::from_ymd_opt(2026, 1, 5).expect("winter date");
        let summer = NaiveDate::from_ymd_opt(2026, 7, 27).expect("summer date");

        assert_eq!(
            market.session_bounds(winter),
            Some((
                Utc.with_ymd_and_hms(2026, 1, 5, 14, 30, 0)
                    .single()
                    .expect("winter open"),
                Utc.with_ymd_and_hms(2026, 1, 5, 21, 0, 0)
                    .single()
                    .expect("winter close"),
            ))
        );
        assert_eq!(
            market.session_bounds(summer),
            Some((
                Utc.with_ymd_and_hms(2026, 7, 27, 13, 30, 0)
                    .single()
                    .expect("summer open"),
                Utc.with_ymd_and_hms(2026, 7, 27, 20, 0, 0)
                    .single()
                    .expect("summer close"),
            ))
        );
    }

    #[test]
    fn session_dates_are_derived_in_the_market_timezone() {
        let market = MarketContext::us_equities();
        let before_midnight_new_york = Utc
            .with_ymd_and_hms(2026, 7, 28, 2, 0, 0)
            .single()
            .expect("timestamp");

        assert_eq!(
            market.session_date(before_midnight_new_york),
            NaiveDate::from_ymd_opt(2026, 7, 27).expect("local date")
        );
    }
}
