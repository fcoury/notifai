use chrono::{DateTime, Local};
use std::collections::HashMap;

use crate::projection::{ProjectedUsage, QuotaProjection};

/// Quota type for tracking notifications
#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub enum QuotaType {
    Session,
    WeekAll,
    WeekSonnet,
}

impl QuotaType {
    pub fn display_name(&self) -> &'static str {
        match self {
            QuotaType::Session => "Session",
            QuotaType::WeekAll => "Week (all models)",
            QuotaType::WeekSonnet => "Week (Sonnet)",
        }
    }
}

/// Notification severity levels
#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub enum NotificationSeverity {
    Approaching, // 100% threshold
    OverBudget,  // 115% threshold
}

/// Tracks which notifications have been sent to avoid duplicates
#[derive(Default)]
pub struct NotificationState {
    /// Track last notification per quota type and severity
    /// Key: (QuotaType, NotificationSeverity), Value: reset_time when notification was sent
    last_notifications: HashMap<(QuotaType, NotificationSeverity), DateTime<Local>>,
}

impl NotificationState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if we should notify for this quota/severity combination
    /// We only notify once per reset period
    pub fn should_notify(
        &self,
        quota: &QuotaType,
        severity: &NotificationSeverity,
        reset_time: DateTime<Local>,
    ) -> bool {
        match self.last_notifications.get(&(quota.clone(), severity.clone())) {
            Some(last_reset_time) => {
                // Only notify if this is a new reset period
                // (the reset time changed since our last notification)
                last_reset_time != &reset_time
            }
            None => true, // Never notified before
        }
    }

    /// Record that we sent a notification
    pub fn record_notification(
        &mut self,
        quota: QuotaType,
        severity: NotificationSeverity,
        reset_time: DateTime<Local>,
    ) {
        self.last_notifications
            .insert((quota, severity), reset_time);
    }
}

/// Check all quotas and return notifications that should be sent
pub fn check_notifications(
    projection: &QuotaProjection,
    state: &NotificationState,
    approaching_threshold: f32,
    over_budget_threshold: f32,
) -> Vec<NotificationInfo> {
    let mut notifications = Vec::new();

    // Helper to check a single quota
    let mut check_quota = |quota_type: QuotaType, proj: &Option<ProjectedUsage>| {
        if let Some(p) = proj {
            // We need reset_time to track notifications per reset period
            // Using projected time as proxy (it's derived from reset_time)
            let now = Local::now();
            // Approximate reset_time from time_remaining_secs
            let reset_time = now + chrono::Duration::seconds(p.time_remaining_secs);

            // Check over budget - higher priority, check first
            if p.projected_percent >= over_budget_threshold {
                if state.should_notify(&quota_type, &NotificationSeverity::OverBudget, reset_time) {
                    notifications.push(NotificationInfo {
                        quota_type: quota_type.clone(),
                        severity: NotificationSeverity::OverBudget,
                        projected_percent: p.projected_percent,
                        reset_time,
                    });
                }
            }
            // Check approaching
            else if p.projected_percent >= approaching_threshold {
                if state.should_notify(&quota_type, &NotificationSeverity::Approaching, reset_time)
                {
                    notifications.push(NotificationInfo {
                        quota_type: quota_type.clone(),
                        severity: NotificationSeverity::Approaching,
                        projected_percent: p.projected_percent,
                        reset_time,
                    });
                }
            }
        }
    };

    check_quota(QuotaType::Session, &projection.session);
    check_quota(QuotaType::WeekAll, &projection.week_all);
    check_quota(QuotaType::WeekSonnet, &projection.week_sonnet);

    notifications
}

/// Information about a notification to send
pub struct NotificationInfo {
    pub quota_type: QuotaType,
    pub severity: NotificationSeverity,
    pub projected_percent: f32,
    pub reset_time: DateTime<Local>,
}

impl NotificationInfo {
    pub fn title(&self) -> String {
        match self.severity {
            NotificationSeverity::Approaching => {
                format!("{} Approaching Budget", self.quota_type.display_name())
            }
            NotificationSeverity::OverBudget => {
                format!("{} Over Budget", self.quota_type.display_name())
            }
        }
    }

    pub fn body(&self) -> String {
        format!("Projected {}% usage at end of period", self.projected_percent as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_state_tracks_correctly() {
        let mut state = NotificationState::new();
        let reset_time = Local::now() + chrono::Duration::hours(2);

        // Should notify first time
        assert!(state.should_notify(
            &QuotaType::Session,
            &NotificationSeverity::Approaching,
            reset_time
        ));

        // Record notification
        state.record_notification(
            QuotaType::Session,
            NotificationSeverity::Approaching,
            reset_time,
        );

        // Should NOT notify again for same reset period
        assert!(!state.should_notify(
            &QuotaType::Session,
            &NotificationSeverity::Approaching,
            reset_time
        ));

        // Should notify for different reset time (new period)
        let new_reset = reset_time + chrono::Duration::hours(5);
        assert!(state.should_notify(
            &QuotaType::Session,
            &NotificationSeverity::Approaching,
            new_reset
        ));
    }
}
