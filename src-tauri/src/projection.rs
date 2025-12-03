use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use chrono_tz::Tz;
use regex::Regex;
use serde::Serialize;

use crate::usage::UsageData;

/// Budget status based on projected usage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BudgetStatus {
    UnderBudget, // projected < 85%
    OnTrack,     // 85% <= projected <= 115%
    OverBudget,  // projected > 115%
    Unknown,     // insufficient data
}

impl BudgetStatus {
    /// Returns the status indicator character
    pub fn indicator(&self) -> &'static str {
        match self {
            BudgetStatus::UnderBudget => "●",
            BudgetStatus::OnTrack => "◐",
            BudgetStatus::OverBudget => "◆",
            BudgetStatus::Unknown => "○",
        }
    }
}

/// Period type for quota calculations
#[derive(Debug, Clone, Copy)]
pub enum PeriodType {
    Session, // 5-hour rolling window
    Weekly,  // 7-day window
}

impl PeriodType {
    fn duration(&self) -> Duration {
        match self {
            PeriodType::Session => Duration::hours(5),
            PeriodType::Weekly => Duration::days(7),
        }
    }
}

/// Projected usage for a single quota
#[derive(Debug, Clone, Serialize)]
pub struct ProjectedUsage {
    pub current_percent: f32,
    pub projected_percent: f32,
    pub status: BudgetStatus,
    pub time_remaining_secs: i64,
}

impl ProjectedUsage {
    /// Format time remaining as human-readable string
    pub fn format_time_remaining(&self) -> String {
        format_duration_secs(self.time_remaining_secs)
    }
}

/// Collection of projections for all quota types
#[derive(Debug, Clone, Serialize)]
pub struct QuotaProjection {
    pub session: Option<ProjectedUsage>,
    pub week_all: Option<ProjectedUsage>,
    pub week_sonnet: Option<ProjectedUsage>,
}

impl QuotaProjection {
    /// Returns the worst status across all quotas
    pub fn worst_status(&self) -> BudgetStatus {
        [&self.session, &self.week_all, &self.week_sonnet]
            .iter()
            .filter_map(|p| p.as_ref())
            .map(|p| p.status)
            .max_by_key(|s| match s {
                BudgetStatus::OverBudget => 3,
                BudgetStatus::OnTrack => 2,
                BudgetStatus::UnderBudget => 1,
                BudgetStatus::Unknown => 0,
            })
            .unwrap_or(BudgetStatus::Unknown)
    }
}

/// Parse reset time strings like:
/// - "6:59pm (America/Sao_Paulo)"
/// - "7pm (America/Sao_Paulo)" - without minutes
/// - "Dec 8 at 3:59pm (America/Sao_Paulo)"
/// - "Dec 8 at 4pm (America/Sao_Paulo)" - without minutes
pub fn parse_reset_time(reset_str: &str) -> Option<DateTime<Local>> {
    // Pattern 1: Time only "6:59pm (timezone)" or "7pm (timezone)" - minutes optional
    let time_only_re =
        Regex::new(r"(\d{1,2})(?::(\d{2}))?\s*(am|pm)\s*\(([^)]+)\)").ok()?;

    // Pattern 2: Date + time "Dec 8 at 3:59pm (timezone)" or "Dec 8 at 4pm (timezone)" - minutes optional
    let date_time_re = Regex::new(
        r"(\w+)\s+(\d{1,2})\s+at\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)\s*\(([^)]+)\)",
    )
    .ok()?;

    // Try date+time format first (more specific)
    if let Some(caps) = date_time_re.captures(reset_str) {
        let month_str = caps.get(1)?.as_str();
        let day: u32 = caps.get(2)?.as_str().parse().ok()?;
        let hour: u32 = caps.get(3)?.as_str().parse().ok()?;
        // Minutes are optional - default to 0 if not present
        let minute: u32 = caps.get(4).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        let am_pm = caps.get(5)?.as_str();
        let tz_str = caps.get(6)?.as_str();

        let hour_24 = to_24_hour(hour, am_pm);
        let month = parse_month(month_str)?;

        // Parse timezone (IANA format like "America/Sao_Paulo")
        let tz: Tz = tz_str.parse().ok()?;

        // Determine year (assume current year, or next year if date has passed)
        let now = Local::now();
        let mut year = now.year();

        // Create naive datetime
        let naive_date = NaiveDate::from_ymd_opt(year, month, day)?;
        let naive_time = NaiveTime::from_hms_opt(hour_24, minute, 0)?;
        let naive_dt = NaiveDateTime::new(naive_date, naive_time);

        // Convert to timezone-aware datetime
        let tz_dt = tz.from_local_datetime(&naive_dt).single()?;

        // If the date is in the past, assume next year
        if tz_dt < now.with_timezone(&tz) {
            year += 1;
            let naive_date = NaiveDate::from_ymd_opt(year, month, day)?;
            let naive_dt = NaiveDateTime::new(naive_date, naive_time);
            let tz_dt = tz.from_local_datetime(&naive_dt).single()?;
            return Some(tz_dt.with_timezone(&Local));
        }

        return Some(tz_dt.with_timezone(&Local));
    }

    // Try time-only format
    if let Some(caps) = time_only_re.captures(reset_str) {
        let hour: u32 = caps.get(1)?.as_str().parse().ok()?;
        // Minutes are optional - default to 0 if not present
        let minute: u32 = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        let am_pm = caps.get(3)?.as_str();
        let tz_str = caps.get(4)?.as_str();

        let hour_24 = to_24_hour(hour, am_pm);

        // Parse timezone (IANA format like "America/Sao_Paulo")
        let tz: Tz = tz_str.parse().ok()?;

        // Use today's date or tomorrow if time has passed
        let now_in_tz = Local::now().with_timezone(&tz);
        let naive_time = NaiveTime::from_hms_opt(hour_24, minute, 0)?;

        // Try today first
        let naive_dt = NaiveDateTime::new(now_in_tz.date_naive(), naive_time);
        let tz_dt = tz.from_local_datetime(&naive_dt).single()?;

        // If time has passed today, use tomorrow
        if tz_dt <= now_in_tz {
            let tomorrow = now_in_tz.date_naive() + Duration::days(1);
            let naive_dt = NaiveDateTime::new(tomorrow, naive_time);
            let tz_dt = tz.from_local_datetime(&naive_dt).single()?;
            return Some(tz_dt.with_timezone(&Local));
        }

        return Some(tz_dt.with_timezone(&Local));
    }

    None
}

/// Convert 12-hour time to 24-hour time
fn to_24_hour(hour: u32, am_pm: &str) -> u32 {
    match (hour, am_pm.to_lowercase().as_str()) {
        (12, "am") => 0,
        (12, "pm") => 12,
        (h, "pm") => h + 12,
        (h, _) => h,
    }
}

/// Parse month name to number
fn parse_month(month_str: &str) -> Option<u32> {
    match month_str.to_lowercase().as_str() {
        "jan" | "january" => Some(1),
        "feb" | "february" => Some(2),
        "mar" | "march" => Some(3),
        "apr" | "april" => Some(4),
        "may" => Some(5),
        "jun" | "june" => Some(6),
        "jul" | "july" => Some(7),
        "aug" | "august" => Some(8),
        "sep" | "september" => Some(9),
        "oct" | "october" => Some(10),
        "nov" | "november" => Some(11),
        "dec" | "december" => Some(12),
        _ => None,
    }
}

/// Calculate projection for a single quota
pub fn calculate_projection(
    current_percent: f32,
    reset_time: DateTime<Local>,
    period_type: PeriodType,
    threshold_under_budget: f32,
    threshold_over_budget: f32,
) -> ProjectedUsage {
    let now = Local::now();
    let period_duration = period_type.duration();
    let period_start = reset_time - period_duration;

    let time_elapsed = now.signed_duration_since(period_start);
    let time_remaining = reset_time.signed_duration_since(now);

    // Edge case: near reset (< 5 minutes remaining)
    if time_remaining < Duration::minutes(5) {
        let status = if current_percent <= 100.0 {
            BudgetStatus::UnderBudget
        } else {
            BudgetStatus::OverBudget
        };
        return ProjectedUsage {
            current_percent,
            projected_percent: current_percent,
            status,
            time_remaining_secs: time_remaining.num_seconds().max(0),
        };
    }

    // Edge case: zero usage
    if current_percent == 0.0 {
        // If less than 10% of period elapsed, mark as unknown
        let elapsed_ratio =
            time_elapsed.num_seconds() as f64 / period_duration.num_seconds() as f64;
        if elapsed_ratio < 0.1 {
            return ProjectedUsage {
                current_percent,
                projected_percent: 0.0,
                status: BudgetStatus::Unknown,
                time_remaining_secs: time_remaining.num_seconds(),
            };
        }
        // Otherwise, project as 0% (under budget)
        return ProjectedUsage {
            current_percent,
            projected_percent: 0.0,
            status: BudgetStatus::UnderBudget,
            time_remaining_secs: time_remaining.num_seconds(),
        };
    }

    // Normal projection: linear extrapolation
    let elapsed_secs = time_elapsed.num_seconds() as f64;
    let total_secs = period_duration.num_seconds() as f64;

    // Avoid division by zero
    if elapsed_secs <= 0.0 {
        return ProjectedUsage {
            current_percent,
            projected_percent: current_percent,
            status: BudgetStatus::Unknown,
            time_remaining_secs: time_remaining.num_seconds(),
        };
    }

    let projected_percent = (current_percent as f64 * total_secs / elapsed_secs) as f32;

    // Determine status based on projected percentage and configurable thresholds
    let status = if projected_percent < threshold_under_budget {
        BudgetStatus::UnderBudget
    } else if projected_percent <= threshold_over_budget {
        BudgetStatus::OnTrack
    } else {
        BudgetStatus::OverBudget
    };

    ProjectedUsage {
        current_percent,
        projected_percent,
        status,
        time_remaining_secs: time_remaining.num_seconds(),
    }
}

/// Calculate projections for all quota types from usage data
pub fn calculate_all_projections(
    usage: &UsageData,
    threshold_under_budget: f32,
    threshold_over_budget: f32,
) -> QuotaProjection {
    let session = usage
        .current_session_percent
        .and_then(|pct| {
            usage
                .current_session_reset
                .as_ref()
                .and_then(|reset| parse_reset_time(reset))
                .map(|reset_time| {
                    calculate_projection(
                        pct,
                        reset_time,
                        PeriodType::Session,
                        threshold_under_budget,
                        threshold_over_budget,
                    )
                })
        });

    let week_all = usage
        .current_week_all_models_percent
        .and_then(|pct| {
            usage
                .current_week_all_models_reset
                .as_ref()
                .and_then(|reset| parse_reset_time(reset))
                .map(|reset_time| {
                    calculate_projection(
                        pct,
                        reset_time,
                        PeriodType::Weekly,
                        threshold_under_budget,
                        threshold_over_budget,
                    )
                })
        });

    let week_sonnet = usage
        .current_week_sonnet_percent
        .and_then(|pct| {
            usage
                .current_week_sonnet_reset
                .as_ref()
                .and_then(|reset| parse_reset_time(reset))
                .map(|reset_time| {
                    calculate_projection(
                        pct,
                        reset_time,
                        PeriodType::Weekly,
                        threshold_under_budget,
                        threshold_over_budget,
                    )
                })
        });

    QuotaProjection {
        session,
        week_all,
        week_sonnet,
    }
}

/// Format duration in seconds to human-readable string
pub fn format_duration_secs(total_seconds: i64) -> String {
    if total_seconds < 0 {
        return "now".to_string();
    }

    let days = total_seconds / 86400;
    let hours = (total_seconds % 86400) / 3600;
    let minutes = (total_seconds % 3600) / 60;

    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_time_only() {
        let result = parse_reset_time("6:59pm (America/Sao_Paulo)");
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_time_only_no_minutes() {
        // Claude sometimes outputs times without minutes like "7pm" instead of "7:00pm"
        let result = parse_reset_time("7pm (America/Sao_Paulo)");
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_date_time() {
        let result = parse_reset_time("Dec 8 at 3:59pm (America/Sao_Paulo)");
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_date_time_no_minutes() {
        // Claude sometimes outputs times without minutes like "4pm" instead of "4:00pm"
        let result = parse_reset_time("Dec 8 at 4pm (America/Sao_Paulo)");
        assert!(result.is_some());
    }

    #[test]
    fn test_status_thresholds() {
        let now = Local::now();
        let reset = now + Duration::hours(2); // 3 hours remaining of 5 hour window

        // 20% used with 2 hours remaining out of 5 hours
        // Elapsed: 3 hours, so projected = 20% * (5/3) = 33.3% -> UnderBudget
        let proj = calculate_projection(20.0, reset, PeriodType::Session, 85.0, 115.0);
        assert_eq!(proj.status, BudgetStatus::UnderBudget);
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration_secs(3600), "1h 0m");
        assert_eq!(format_duration_secs(86400 + 3600), "1d 1h");
        assert_eq!(format_duration_secs(300), "5m");
        assert_eq!(format_duration_secs(-10), "now");
    }
}
