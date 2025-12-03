mod usage;

use std::sync::{Arc, Mutex};
use std::thread;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle,
};

fn build_usage_menu(app: &AppHandle, data: Option<&usage::UsageData>) -> Menu<tauri::Wry> {
    let menu = Menu::new(app).unwrap();

    if let Some(usage) = data {
        // Session usage
        if let Some(pct) = usage.current_session_percent {
            let reset = usage.current_session_reset.as_deref().unwrap_or("unknown");
            let item = MenuItem::with_id(
                app,
                "session",
                format!("Session: {}% (resets {})", pct, reset),
                false,
                None::<&str>,
            )
            .unwrap();
            let _ = menu.append(&item);
        }

        // Week (all models) usage
        if let Some(pct) = usage.current_week_all_models_percent {
            let reset = usage
                .current_week_all_models_reset
                .as_deref()
                .unwrap_or("unknown");
            let item = MenuItem::with_id(
                app,
                "week_all",
                format!("Week (all): {}% (resets {})", pct, reset),
                false,
                None::<&str>,
            )
            .unwrap();
            let _ = menu.append(&item);
        }

        // Week (Sonnet) usage
        if let Some(pct) = usage.current_week_sonnet_percent {
            let reset = usage
                .current_week_sonnet_reset
                .as_deref()
                .unwrap_or("unknown");
            let item = MenuItem::with_id(
                app,
                "week_sonnet",
                format!("Week (Sonnet): {}% (resets {})", pct, reset),
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

    let refresh = MenuItem::with_id(app, "refresh", "Refresh", true, None::<&str>).unwrap();
    let _ = menu.append(&refresh);

    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>).unwrap();
    let _ = menu.append(&quit);

    menu
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let usage_data: Arc<Mutex<Option<usage::UsageData>>> = Arc::new(Mutex::new(None));
    let usage_data_clone = usage_data.clone();

    tauri::Builder::default()
        .setup(move |app| {
            // Hide from dock on macOS
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_handle = app.handle().clone();
            let data = usage_data_clone.clone();

            // Build initial tray with loading state
            let menu = build_usage_menu(&app_handle, None);

            let _tray = TrayIconBuilder::with_id("main")
                .icon(tauri::include_image!("icons/icon.png"))
                .icon_as_template(true)
                .tooltip("NotifAI - Claude Usage")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "refresh" => {
                        let data = data.clone();
                        let app = app.clone();
                        thread::spawn(move || {
                            if let Ok(usage) = usage::fetch_usage() {
                                *data.lock().unwrap() = Some(usage.clone());
                                let menu = build_usage_menu(&app, Some(&usage));
                                if let Some(tray) = app.tray_by_id("main") {
                                    let _ = tray.set_menu(Some(menu));
                                }
                            }
                        });
                    }
                    _ => {}
                })
                .build(app)?;

            // Initial fetch in background
            let app_handle = app.handle().clone();
            let data = usage_data.clone();
            thread::spawn(move || {
                if let Ok(usage) = usage::fetch_usage() {
                    *data.lock().unwrap() = Some(usage.clone());
                    let menu = build_usage_menu(&app_handle, Some(&usage));
                    if let Some(tray) = app_handle.tray_by_id("main") {
                        let _ = tray.set_menu(Some(menu));
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
