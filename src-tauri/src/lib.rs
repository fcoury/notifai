mod notification;
mod projection;
mod settings;
mod usage;

use chrono::{DateTime, Local};
use notification::{check_notifications, NotificationState};
use projection::{calculate_all_projections, format_duration_secs, BudgetStatus, QuotaProjection};
use settings::{load_settings, save_settings, Settings};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_notification::NotificationExt;

/// Application state
struct AppState {
    usage: Option<usage::UsageData>,
    projection: Option<QuotaProjection>,
    last_refresh: Option<DateTime<Local>>,
    is_refreshing: AtomicBool,
    settings: Settings,
}

impl AppState {
    fn new() -> Self {
        Self {
            usage: None,
            projection: None,
            last_refresh: None,
            is_refreshing: AtomicBool::new(false),
            settings: Settings::default(),
        }
    }
}

fn build_usage_menu(app: &AppHandle, state: &AppState) -> Menu<tauri::Wry> {
    let menu = Menu::new(app).unwrap();

    if let (Some(usage), Some(proj)) = (&state.usage, &state.projection) {
        // Session usage with projection
        if let Some(session) = &proj.session {
            let indicator = session.status.indicator();
            let time_remaining = session.format_time_remaining();
            let item = MenuItem::with_id(
                app,
                "session",
                format!(
                    "{} Session: {}% → {}% (resets in {})",
                    indicator,
                    session.current_percent as i32,
                    session.projected_percent as i32,
                    time_remaining
                ),
                false,
                None::<&str>,
            )
            .unwrap();
            let _ = menu.append(&item);
        }

        // Week (all models) with projection
        if let Some(week_all) = &proj.week_all {
            let indicator = week_all.status.indicator();
            let time_remaining = week_all.format_time_remaining();
            let item = MenuItem::with_id(
                app,
                "week_all",
                format!(
                    "{} Week (all): {}% → {}% (resets in {})",
                    indicator,
                    week_all.current_percent as i32,
                    week_all.projected_percent as i32,
                    time_remaining
                ),
                false,
                None::<&str>,
            )
            .unwrap();
            let _ = menu.append(&item);
        }

        // Week (Sonnet) with projection
        if let Some(week_sonnet) = &proj.week_sonnet {
            let indicator = week_sonnet.status.indicator();
            let time_remaining = week_sonnet.format_time_remaining();
            let item = MenuItem::with_id(
                app,
                "week_sonnet",
                format!(
                    "{} Week (Sonnet): {}% → {}% (resets in {})",
                    indicator,
                    week_sonnet.current_percent as i32,
                    week_sonnet.projected_percent as i32,
                    time_remaining
                ),
                false,
                None::<&str>,
            )
            .unwrap();
            let _ = menu.append(&item);
        }

        // Extra usage status
        let extra_text = if usage.extra_usage_enabled {
            "enabled"
        } else {
            "not enabled"
        };
        let extra = MenuItem::with_id(
            app,
            "extra",
            format!("Extra usage: {}", extra_text),
            false,
            None::<&str>,
        )
        .unwrap();
        let _ = menu.append(&extra);
    } else {
        let loading =
            MenuItem::with_id(app, "loading", "Loading...", false, None::<&str>).unwrap();
        let _ = menu.append(&loading);
    }

    // Separator and actions
    let _ = menu.append(&PredefinedMenuItem::separator(app).unwrap());

    // Show last updated time
    if let Some(last_refresh) = &state.last_refresh {
        let elapsed = Local::now().signed_duration_since(*last_refresh);
        let ago_text = if elapsed.num_seconds() < 60 {
            "just now".to_string()
        } else {
            format!("{} ago", format_duration_secs(elapsed.num_seconds()))
        };
        let last_updated = MenuItem::with_id(
            app,
            "last_updated",
            format!("Updated {}", ago_text),
            false,
            None::<&str>,
        )
        .unwrap();
        let _ = menu.append(&last_updated);
        let _ = menu.append(&PredefinedMenuItem::separator(app).unwrap());
    }

    // Settings item with Cmd+, accelerator
    let settings_item = MenuItem::with_id(
        app,
        "settings",
        "Settings...",
        true,
        Some("CmdOrCtrl+,"),
    )
    .unwrap();
    let _ = menu.append(&settings_item);

    // About item
    let about = MenuItem::with_id(app, "about", "About NotifAI", true, None::<&str>).unwrap();
    let _ = menu.append(&about);

    let _ = menu.append(&PredefinedMenuItem::separator(app).unwrap());

    let refresh = MenuItem::with_id(app, "refresh", "Refresh", true, None::<&str>).unwrap();
    let _ = menu.append(&refresh);

    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>).unwrap();
    let _ = menu.append(&quit);

    menu
}

/// Get the appropriate icon for the given status
fn get_status_icon(status: BudgetStatus) -> Image<'static> {
    match status {
        BudgetStatus::UnderBudget => tauri::include_image!("icons/icon-green.png"),
        BudgetStatus::OnTrack => tauri::include_image!("icons/icon-yellow.png"),
        BudgetStatus::OverBudget => tauri::include_image!("icons/icon-red.png"),
        BudgetStatus::Unknown => tauri::include_image!("icons/icon-gray.png"),
    }
}

/// Update the tray icon based on the worst status
fn update_tray_icon(tray: &TrayIcon, status: BudgetStatus) {
    let icon = get_status_icon(status);
    let _ = tray.set_icon(Some(icon));
}

/// Open the settings window
fn open_settings_window(app: &AppHandle) {
    // Check if window already exists
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }

    // Create new window
    let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("NotifAI Settings")
        .inner_size(480.0, 420.0)
        .resizable(false)
        .center()
        .build();
}

/// Open the about window
fn open_about_window(app: &AppHandle) {
    // Check if window already exists
    if let Some(window) = app.get_webview_window("about") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }

    // Create new window
    let _ = WebviewWindowBuilder::new(app, "about", WebviewUrl::App("about.html".into()))
        .title("About NotifAI")
        .inner_size(300.0, 220.0)
        .resizable(false)
        .center()
        .build();
}

/// Fetch usage and update state
fn fetch_and_update(
    app: &AppHandle,
    state: &Arc<Mutex<AppState>>,
    notif_state: &Arc<Mutex<NotificationState>>,
) {
    eprintln!("[NotifAI] fetch_and_update called");

    // Get current settings for notification checks
    let current_settings = {
        let guard = state.lock().unwrap();
        guard.settings.clone()
    };

    match usage::fetch_usage() {
        Ok(usage) => {
            eprintln!("[NotifAI] fetch_usage succeeded");
            eprintln!("[NotifAI] Usage data: session={:?}, week_all={:?}, week_sonnet={:?}",
                usage.current_session_percent,
                usage.current_week_all_models_percent,
                usage.current_week_sonnet_percent
            );
            eprintln!("[NotifAI] Reset times: session={:?}, week_all={:?}, week_sonnet={:?}",
                usage.current_session_reset,
                usage.current_week_all_models_reset,
                usage.current_week_sonnet_reset
            );
        let projection = calculate_all_projections(
            &usage,
            current_settings.threshold_under_budget,
            current_settings.threshold_on_track,
        );
        eprintln!("[NotifAI] Projection: session={:?}, week_all={:?}, week_sonnet={:?}",
            projection.session.as_ref().map(|p| p.projected_percent),
            projection.week_all.as_ref().map(|p| p.projected_percent),
            projection.week_sonnet.as_ref().map(|p| p.projected_percent)
        );
        let worst_status = projection.worst_status();
        eprintln!("[NotifAI] Worst status: {:?}", worst_status);

        // Check and send notifications (if enabled)
        if current_settings.notifications_enabled {
            let notif_guard = notif_state.lock().unwrap();
            let notifications = check_notifications(
                &projection,
                &notif_guard,
                current_settings.notify_approaching_percent,
                current_settings.notify_over_budget_percent,
            );
            drop(notif_guard);

            for info in notifications {
                // Send the notification
                let _ = app
                    .notification()
                    .builder()
                    .title(&info.title())
                    .body(&info.body())
                    .show();

                // Record that we sent it
                let mut notif_guard = notif_state.lock().unwrap();
                notif_guard.record_notification(info.quota_type, info.severity, info.reset_time);
            }
        }

        // Update state
        {
            let mut state_guard = state.lock().unwrap();
            state_guard.usage = Some(usage);
            state_guard.projection = Some(projection);
            state_guard.last_refresh = Some(Local::now());
            eprintln!("[NotifAI] State updated successfully");
        }

        // Update menu
        let state_guard = state.lock().unwrap();
        eprintln!("[NotifAI] Building menu with state: usage={}, projection={}",
            state_guard.usage.is_some(),
            state_guard.projection.is_some()
        );
        let menu = build_usage_menu(app, &state_guard);
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_menu(Some(menu));
            // Update icon based on status
            update_tray_icon(&tray, worst_status);
            eprintln!("[NotifAI] Menu and icon updated");
        } else {
            eprintln!("[NotifAI] ERROR: Could not find tray with id 'main'");
        }
        }
        Err(e) => {
            eprintln!("[NotifAI] fetch_usage failed: {}", e);
        }
    }
}

/// Start the auto-refresh background loop
fn start_auto_refresh(
    app: AppHandle,
    state: Arc<Mutex<AppState>>,
    notif_state: Arc<Mutex<NotificationState>>,
) {
    thread::spawn(move || {
        loop {
            // Get current interval from settings
            let interval_minutes = {
                let guard = state.lock().unwrap();
                guard.settings.refresh_interval_minutes
            };
            let interval = Duration::from_secs(interval_minutes * 60);

            thread::sleep(interval);

            // Check if already refreshing
            {
                let state_guard = state.lock().unwrap();
                if state_guard.is_refreshing.swap(true, Ordering::SeqCst) {
                    continue; // Skip this cycle if already refreshing
                }
            }

            // Do the refresh
            fetch_and_update(&app, &state, &notif_state);

            // Mark as done refreshing
            {
                let state_guard = state.lock().unwrap();
                state_guard.is_refreshing.store(false, Ordering::SeqCst);
            }
        }
    });
}

// Tauri commands for settings

#[tauri::command]
fn get_settings(state: tauri::State<'_, Arc<Mutex<AppState>>>) -> Settings {
    let guard = state.lock().unwrap();
    guard.settings.clone()
}

#[tauri::command]
fn save_settings_cmd(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Mutex<AppState>>>,
    new_settings: Settings,
) -> Result<(), String> {
    // Validate
    new_settings.validate().map_err(|e| e.join(", "))?;

    // Save to store
    save_settings(&app, &new_settings)?;

    // Update in-memory state
    let mut guard = state.lock().unwrap();
    guard.settings = new_settings;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state: Arc<Mutex<AppState>> = Arc::new(Mutex::new(AppState::new()));
    let notif_state: Arc<Mutex<NotificationState>> =
        Arc::new(Mutex::new(NotificationState::new()));

    let state_for_setup = app_state.clone();
    let notif_for_setup = notif_state.clone();
    let state_for_invoke = app_state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .manage(state_for_invoke)
        .invoke_handler(tauri::generate_handler![get_settings, save_settings_cmd])
        .setup(move |app| {
            // Hide from dock on macOS
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_handle = app.handle().clone();

            // Load settings from store
            let loaded_settings = load_settings(&app_handle);
            {
                let mut guard = state_for_setup.lock().unwrap();
                guard.settings = loaded_settings;
            }

            let state = state_for_setup.clone();
            let notif = notif_for_setup.clone();

            // Build initial tray with loading state
            let initial_state = state.lock().unwrap();
            let menu = build_usage_menu(&app_handle, &initial_state);
            drop(initial_state);

            let state_for_events = state.clone();
            let notif_for_events = notif.clone();
            let app_for_events = app_handle.clone();

            let _tray = TrayIconBuilder::with_id("main")
                .icon(tauri::include_image!("icons/icon-gray.png"))
                .tooltip("NotifAI - Claude Usage")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "refresh" => {
                        let state = state_for_events.clone();
                        let notif = notif_for_events.clone();
                        let app = app.clone();
                        thread::spawn(move || {
                            fetch_and_update(&app, &state, &notif);
                        });
                    }
                    "settings" => {
                        open_settings_window(&app_for_events);
                    }
                    "about" => {
                        open_about_window(&app_for_events);
                    }
                    _ => {}
                })
                .build(app)?;

            // Initial fetch in background
            let app_handle_for_fetch = app.handle().clone();
            let state_for_fetch = state.clone();
            let notif_for_fetch = notif.clone();
            thread::spawn(move || {
                fetch_and_update(&app_handle_for_fetch, &state_for_fetch, &notif_for_fetch);
            });

            // Start auto-refresh loop
            let app_handle_for_refresh = app.handle().clone();
            let state_for_refresh = state.clone();
            let notif_for_refresh = notif.clone();
            start_auto_refresh(app_handle_for_refresh, state_for_refresh, notif_for_refresh);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
