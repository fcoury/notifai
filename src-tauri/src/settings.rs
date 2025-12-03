use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

/// Refresh interval options (in minutes)
pub const REFRESH_INTERVALS: [u64; 4] = [5, 15, 30, 60];

/// Default settings values
pub mod defaults {
    pub const REFRESH_INTERVAL_MINUTES: u64 = 15;
    pub const THRESHOLD_UNDER_BUDGET: f32 = 85.0;
    pub const THRESHOLD_ON_TRACK: f32 = 115.0;
    pub const NOTIFICATIONS_ENABLED: bool = true;
    pub const NOTIFY_APPROACHING_PERCENT: f32 = 100.0;
    pub const NOTIFY_OVER_BUDGET_PERCENT: f32 = 115.0;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub refresh_interval_minutes: u64,
    pub threshold_under_budget: f32,
    pub threshold_on_track: f32,
    pub notifications_enabled: bool,
    pub notify_approaching_percent: f32,
    pub notify_over_budget_percent: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh_interval_minutes: defaults::REFRESH_INTERVAL_MINUTES,
            threshold_under_budget: defaults::THRESHOLD_UNDER_BUDGET,
            threshold_on_track: defaults::THRESHOLD_ON_TRACK,
            notifications_enabled: defaults::NOTIFICATIONS_ENABLED,
            notify_approaching_percent: defaults::NOTIFY_APPROACHING_PERCENT,
            notify_over_budget_percent: defaults::NOTIFY_OVER_BUDGET_PERCENT,
        }
    }
}

impl Settings {
    /// Validate settings and return errors if invalid
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if !REFRESH_INTERVALS.contains(&self.refresh_interval_minutes) {
            errors.push(format!(
                "Refresh interval must be one of: {:?}",
                REFRESH_INTERVALS
            ));
        }

        if self.threshold_under_budget < 1.0 || self.threshold_under_budget > 99.0 {
            errors.push("Under budget threshold must be between 1 and 99".to_string());
        }

        if self.threshold_on_track < 2.0 || self.threshold_on_track > 200.0 {
            errors.push("On track threshold must be between 2 and 200".to_string());
        }

        if self.threshold_under_budget >= self.threshold_on_track {
            errors.push("Under budget must be less than on track threshold".to_string());
        }

        if self.notify_approaching_percent < 1.0 || self.notify_approaching_percent > 200.0 {
            errors.push("Approaching notification must be between 1 and 200".to_string());
        }

        if self.notify_over_budget_percent < 1.0 || self.notify_over_budget_percent > 200.0 {
            errors.push("Over budget notification must be between 1 and 200".to_string());
        }

        if self.notify_over_budget_percent < self.notify_approaching_percent {
            errors
                .push("Over budget notification must be >= approaching notification".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Load settings from store, falling back to defaults
pub fn load_settings(app: &AppHandle) -> Settings {
    let store = match app.store("settings.json") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open settings store: {}", e);
            return Settings::default();
        }
    };

    // Try to load each field individually, falling back to defaults
    let defaults = Settings::default();

    let settings = Settings {
        refresh_interval_minutes: store
            .get("refresh_interval_minutes")
            .and_then(|v| v.as_u64())
            .unwrap_or(defaults.refresh_interval_minutes),
        threshold_under_budget: store
            .get("threshold_under_budget")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(defaults.threshold_under_budget),
        threshold_on_track: store
            .get("threshold_on_track")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(defaults.threshold_on_track),
        notifications_enabled: store
            .get("notifications_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(defaults.notifications_enabled),
        notify_approaching_percent: store
            .get("notify_approaching_percent")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(defaults.notify_approaching_percent),
        notify_over_budget_percent: store
            .get("notify_over_budget_percent")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(defaults.notify_over_budget_percent),
    };

    // Validate loaded settings, use defaults if invalid
    match settings.validate() {
        Ok(_) => settings,
        Err(errors) => {
            eprintln!("Invalid settings loaded, using defaults: {:?}", errors);
            Settings::default()
        }
    }
}

/// Save settings to store
pub fn save_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    // Validate before saving
    settings.validate().map_err(|e| e.join(", "))?;

    let store = app.store("settings.json").map_err(|e| e.to_string())?;

    store.set(
        "refresh_interval_minutes",
        json!(settings.refresh_interval_minutes),
    );
    store.set(
        "threshold_under_budget",
        json!(settings.threshold_under_budget),
    );
    store.set("threshold_on_track", json!(settings.threshold_on_track));
    store.set("notifications_enabled", json!(settings.notifications_enabled));
    store.set(
        "notify_approaching_percent",
        json!(settings.notify_approaching_percent),
    );
    store.set(
        "notify_over_budget_percent",
        json!(settings.notify_over_budget_percent),
    );

    store.save().map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings_are_valid() {
        let settings = Settings::default();
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_invalid_refresh_interval() {
        let mut settings = Settings::default();
        settings.refresh_interval_minutes = 10; // Invalid
        assert!(settings.validate().is_err());
    }

    #[test]
    fn test_invalid_threshold_relationship() {
        let mut settings = Settings::default();
        settings.threshold_under_budget = 120.0;
        settings.threshold_on_track = 100.0;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn test_invalid_notification_relationship() {
        let mut settings = Settings::default();
        settings.notify_approaching_percent = 120.0;
        settings.notify_over_budget_percent = 100.0;
        assert!(settings.validate().is_err());
    }
}
