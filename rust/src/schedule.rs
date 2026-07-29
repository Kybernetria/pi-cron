use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::Cron;
use std::str::FromStr;

pub fn next_occurrence(
    expression: &str,
    timezone: &str,
    after: DateTime<Utc>,
) -> Result<DateTime<Utc>, String> {
    if expression.split_whitespace().count() != 5 {
        return Err("schedule must be a five-field cron expression".into());
    }
    let tz = Tz::from_str(timezone).map_err(|_| format!("Invalid IANA timezone: {timezone}"))?;
    let schedule = Cron::from_str(expression).map_err(|e| format!("Invalid cron schedule: {e}"))?;
    schedule
        .find_next_occurrence(&after.with_timezone(&tz), false)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|e| format!("Invalid cron schedule: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dst_spring_forward() {
        let after = "2025-03-09T12:00:00Z".parse().unwrap();
        assert_eq!(
            next_occurrence("0 9 * * *", "America/New_York", after)
                .unwrap()
                .to_rfc3339(),
            "2025-03-09T13:00:00+00:00"
        );
    }
    #[test]
    fn dst_fall_back_selects_a_real_future_instant() {
        let after = "2025-11-02T04:00:00Z".parse().unwrap();
        assert_eq!(
            next_occurrence("30 1 * * *", "America/New_York", after)
                .unwrap()
                .to_rfc3339(),
            "2025-11-02T05:30:00+00:00"
        );
    }
    #[test]
    fn follows_standard_weekday_and_dom_or_dow_semantics() {
        let after = "2025-01-03T10:00:00Z".parse().unwrap();
        assert_eq!(
            next_occurrence("0 9 * * 1-5", "UTC", after)
                .unwrap()
                .to_rfc3339(),
            "2025-01-06T09:00:00+00:00"
        );
        let after = "2025-01-02T00:00:00Z".parse().unwrap();
        assert_eq!(
            next_occurrence("0 9 1 * 1", "UTC", after)
                .unwrap()
                .to_rfc3339(),
            "2025-01-06T09:00:00+00:00"
        );
        assert_eq!(
            next_occurrence("0 9 * * 0", "UTC", after)
                .unwrap()
                .to_rfc3339(),
            "2025-01-05T09:00:00+00:00"
        );
    }
    #[test]
    fn rejects_non_five_field_and_bad_zone() {
        let now = Utc::now();
        assert!(
            next_occurrence("0 9 * *", "UTC", now)
                .unwrap_err()
                .contains("five-field")
        );
        assert!(
            next_occurrence("0 9 * * *", "Mars/Olympus", now)
                .unwrap_err()
                .contains("IANA")
        );
    }
}
