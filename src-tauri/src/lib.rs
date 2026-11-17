use std::path::PathBuf;

use hint_core::{llm, secret, settings::Settings};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, State,
};
use tauri_plugin_global_shortcut::{Code, Modifiers, ShortcutState};

struct AppState {
    settings_path: PathBuf,
}

fn toggle_overlay(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("overlay") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    Settings::load(&state.settings_path)
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<(), String> {
    settings.save(&state.settings_path).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_api_key(provider: String, key: String) -> Result<(), String> {
    secret::save(&provider, &key).map_err(|e| e.to_string())
}

#[tauri::command]
fn has_api_key(provider: String) -> bool {
    secret::has(&provider)
}

#[tauri::command]
async fn test_llm(
    state: State<'_, AppState>,
    prompt: String,
) -> Result<llm::ChatResponse, String> {
    let settings = Settings::load(&state.settings_path);
    let provider = settings.provider.as_str().to_string();
    let key = secret::get(&provider).map_err(|e| e.to_string())?;
    let system = settings.system_prompt();
    llm::complete(
        &settings,
        &key,
        llm::ChatRequest {
            system,
            user: prompt,
        },
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn toggle_overlay_cmd(app: tauri::AppHandle) -> bool {
    toggle_overlay(&app);
    app.get_webview_window("overlay")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts(["ctrl+shift+space"])
                .expect("注册全局热键失败")
                .with_handler(|app, shortcut, event| {
                    if event.state == ShortcutState::Pressed
                        && shortcut.matches(Modifiers::CONTROL | Modifiers::SHIFT, Code::Space)
                    {
                        toggle_overlay(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            app.manage(AppState {
                settings_path: data_dir.join("settings.json"),
            });

            let show = MenuItem::with_id(app, "show", "打开设置", true, None::<&str>)?;
            let overlay = MenuItem::with_id(app, "overlay", "显示 / 隐藏浮层", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&show, &overlay, &separator, &quit])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("hint · 面试提词器")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "overlay" => toggle_overlay(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            save_api_key,
            has_api_key,
            test_llm,
            toggle_overlay_cmd
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
