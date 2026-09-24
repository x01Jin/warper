mod aumid;
mod vault;
mod warp;

use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use warp::{WarpIdentity, WarpInfo, WarpStatus};

pub(crate) struct WarpLock(pub(crate) Arc<tokio::sync::Mutex<()>>);

pub(crate) struct AutoState(pub(crate) Mutex<Instant>);

pub(crate) struct WindowBuildLock(pub(crate) Mutex<()>);

pub(crate) struct VaultWarned(pub(crate) Mutex<bool>);

fn note_op(app: &AppHandle) {
    if let Ok(mut last) = app.state::<AutoState>().0.lock() {
        *last = Instant::now();
    }
}

pub(crate) const SETTINGS_FILE: &str = "warper-config.json";
pub(crate) const LOG_FILE: &str = "warper.log";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsLocation {
    path: String,
    portable: bool,
}

pub(crate) fn settings_path(app: &AppHandle) -> (std::path::PathBuf, bool) {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let probe = dir.join(".warper-write-test");
            if std::fs::write(&probe, b"").is_ok() {
                let _ = std::fs::remove_file(&probe);
                return (dir.join(SETTINGS_FILE), true);
            }
        }
    }
    let fallback = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(SETTINGS_FILE);
    (fallback, false)
}

pub(crate) fn log_path(app: &AppHandle) -> std::path::PathBuf {
    let (settings, _) = settings_path(app);
    settings.with_file_name(LOG_FILE)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    step: String,
}

pub(crate) fn emit(app: &AppHandle, step: &str) {
    let _ = app.emit(
        "warp-progress",
        Progress {
            step: step.to_string(),
        },
    );
    vault_push(app, step);
}

fn vault_warn(app: &AppHandle, message: &str) {
    if let Ok(mut warned) = app.state::<VaultWarned>().0.lock() {
        if !*warned {
            *warned = true;
            emit(app, message);
        }
    }
}

fn vault_save(app: &AppHandle, data: &vault::VaultData) {
    if vault::save(&log_path(app), data).is_err() {
        vault_warn(app, "log save failed");
    }
}

fn vault_push(app: &AppHandle, message: &str) {
    let mut data = vault::load(&log_path(app));
    vault::push_line_data(&mut data, message);
    vault_save(app, &data);
}

fn vault_clear(app: &AppHandle) {
    let mut data = vault::load(&log_path(app));
    data.lines.clear();
    vault_save(app, &data);
}

fn read_history(app: &AppHandle) -> Vec<vault::SealedEntry> {
    use tauri_plugin_store::StoreExt;
    let (path, _) = settings_path(app);
    app.store(path)
        .ok()
        .and_then(|store| store.get("ipHistory"))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn write_history(app: &AppHandle, list: &[vault::SealedEntry]) {
    use tauri_plugin_store::StoreExt;
    let (path, _) = settings_path(app);
    let Some(store) = app.store(path).ok() else {
        vault_warn(app, "history save failed");
        return;
    };
    let Ok(value) = serde_json::to_value(list) else {
        vault_warn(app, "history save failed");
        return;
    };
    store.set("ipHistory", value);
    if store.save().is_err() {
        vault_warn(app, "history save failed");
    }
}

fn record_history(app: &AppHandle, ip: &str) -> bool {
    if ip.is_empty() || ip == "unreachable" {
        return false;
    }
    let list = read_history(app);
    if list
        .first()
        .and_then(|e| vault::open_ip(&e.enc))
        .is_some_and(|first| first == ip)
    {
        return false;
    }
    match vault::next_history(&list, ip, 0) {
        Some(next) => {
            write_history(app, &next);
            true
        }
        None => {
            vault_warn(app, "log save failed");
            false
        }
    }
}

fn migrate_history_once(app: &AppHandle) {
    let data = vault::load(&log_path(app));
    if data.history.is_empty() {
        return;
    }
    if read_history(app).is_empty() {
        write_history(app, &data.history);
    }
    let mut data = data;
    data.history.clear();
    vault_save(app, &data);
}

fn notifications_allowed(app: &AppHandle) -> bool {
    use tauri_plugin_store::StoreExt;
    let (path, _) = settings_path(app);
    app.store(path)
        .ok()
        .and_then(|store| store.get("notifications"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn toast(app: &AppHandle, body: &str) {
    if !notifications_allowed(app) {
        return;
    }
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title("Warper")
        .body(body)
        .show();
}

async fn refresh_ip(app: &AppHandle) -> String {
    let ip = warp::fetch_exit_ip().await;
    if record_history(app, &ip) {
        vault_push(app, "Exit IP changed");
        toast(app, "Exit IP changed");
    }
    ip
}

async fn emit_ip(app: &AppHandle) {
    let ip = refresh_ip(app).await;
    let _ = app.emit("warp-ip", ip);
}

fn spawn_ip_watch(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            emit_ip(&app).await;
        }
    });
}

async fn locked(state: State<'_, WarpLock>) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    state
        .0
        .clone()
        .try_lock_owned()
        .map_err(|_| "another operation is already running".to_string())
}

async fn fresh_registration(app: &AppHandle) -> Result<(), String> {
    match warp::run_warp(&["registration", "new"]).await {
        Ok(_) => Ok(()),
        Err(error) if error.contains("still around") => {
            emit(app, "clearing stale registration…");
            warp::run_warp(&["registration", "delete"]).await?;
            warp::sleep_ms(500).await;
            warp::run_warp(&["registration", "new"]).await?;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

async fn connect_with_mode(app: &AppHandle, mode: &str) -> Result<(), String> {
    emit(app, &format!("setting mode to {mode}…"));
    warp::set_mode(mode).await?;
    emit(app, "connecting…");
    warp::run_warp(&["connect"]).await?;
    Ok(())
}

async fn run_quick_reset(app: &AppHandle, connect_mode: &str) -> Result<String, String> {
    emit(app, "[1/3] disconnecting…");
    warp::run_warp(&["disconnect"]).await?;
    warp::sleep_ms(1000).await;
    emit(app, "[2/3] registering new tunnel…");
    fresh_registration(app).await?;
    warp::sleep_ms(1000).await;
    emit(app, "[3/3] connecting…");
    connect_with_mode(app, connect_mode).await?;
    emit(app, "quick reset complete");
    Ok("quick reset complete".to_string())
}

async fn run_full_reset(app: &AppHandle, connect_mode: &str) -> Result<String, String> {
    emit(app, "[1/4] disconnecting…");
    warp::run_warp(&["disconnect"]).await?;
    warp::sleep_ms(1000).await;
    emit(app, "[2/4] deleting registration…");
    warp::run_warp(&["registration", "delete"]).await?;
    warp::sleep_ms(500).await;
    emit(app, "[3/4] registering new tunnel…");
    fresh_registration(app).await?;
    warp::sleep_ms(1000).await;
    emit(app, "[4/4] connecting…");
    connect_with_mode(app, connect_mode).await?;
    emit(app, "full reset complete");
    Ok("full reset complete".to_string())
}

#[tauri::command]
async fn get_status() -> Result<WarpStatus, String> {
    warp::get_status().await
}

#[tauri::command]
async fn get_identity() -> Result<WarpIdentity, String> {
    warp::get_identity().await
}

#[tauri::command]
async fn get_warp_info() -> WarpInfo {
    warp::warp_info().await
}

#[tauri::command]
async fn get_settings_path(app: AppHandle) -> SettingsLocation {
    let (path, portable) = settings_path(&app);
    SettingsLocation {
        path: path.to_string_lossy().into_owned(),
        portable,
    }
}

#[tauri::command]
async fn get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_autostart(app: AppHandle, enabled: bool) -> Result<String, String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())?;
        Ok("autostart enabled — Warper launches on login".to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())?;
        Ok("autostart disabled".to_string())
    }
}

#[tauri::command]
async fn get_mode() -> Result<String, String> {
    warp::get_mode().await
}

#[tauri::command]
async fn warp_connect(
    app: AppHandle,
    state: State<'_, WarpLock>,
    connect_mode: String,
) -> Result<String, String> {
    let _guard = locked(state).await?;
    connect_with_mode(&app, connect_mode.trim()).await?;
    emit(&app, "connected");
    emit_ip(&app).await;
    note_op(&app);
    Ok("WARP connected".to_string())
}

#[tauri::command]
async fn warp_standby(
    app: AppHandle,
    state: State<'_, WarpLock>,
    standby: String,
) -> Result<String, String> {
    let _guard = locked(state).await?;
    let standby = standby.trim();
    if standby == "off" {
        emit(&app, "disconnecting…");
        warp::run_warp(&["disconnect"]).await?;
        emit(&app, "disconnected");
        emit_ip(&app).await;
        note_op(&app);
        Ok("WARP disconnected".to_string())
    } else {
        emit(&app, &format!("switching to {standby} mode…"));
        warp::set_mode(standby).await?;
        emit(&app, &format!("standby mode ({standby})"));
        emit_ip(&app).await;
        note_op(&app);
        Ok(format!("WARP on standby ({standby})"))
    }
}

#[tauri::command]
async fn quick_reset(
    app: AppHandle,
    state: State<'_, WarpLock>,
    connect_mode: String,
) -> Result<String, String> {
    let _guard = locked(state).await?;
    let result = run_quick_reset(&app, connect_mode.trim()).await;
    if result.is_ok() {
        emit_ip(&app).await;
    }
    note_op(&app);
    result
}

#[tauri::command]
async fn full_reset(
    app: AppHandle,
    state: State<'_, WarpLock>,
    connect_mode: String,
) -> Result<String, String> {
    let _guard = locked(state).await?;
    let result = run_full_reset(&app, connect_mode.trim()).await;
    if result.is_ok() {
        emit_ip(&app).await;
    }
    note_op(&app);
    result
}

#[tauri::command]
async fn auto_countdown(app: AppHandle) -> i64 {
    use tauri_plugin_store::StoreExt;
    let (path, _) = settings_path(&app);
    let Ok(store) = app.store(path) else {
        return -1;
    };
    let enabled = store
        .get("autoReset")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !enabled {
        return -1;
    }
    let mins = store
        .get("intervalMinutes")
        .and_then(|v| v.as_u64())
        .unwrap_or(15);
    let elapsed = app
        .state::<AutoState>()
        .0
        .lock()
        .map(|last| last.elapsed().as_secs())
        .unwrap_or(0);
    (mins.saturating_mul(60).saturating_sub(elapsed)) as i64
}

#[tauri::command]
async fn reset_auto_timer(app: AppHandle) {
    note_op(&app);
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    ip: String,
    at: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LogState {
    history: Vec<HistoryEntry>,
    lines: Vec<String>,
}

#[tauri::command]
async fn get_log_state(app: AppHandle) -> LogState {
    let history = read_history(&app)
        .iter()
        .filter_map(|e| vault::open_ip(&e.enc).map(|ip| HistoryEntry { ip, at: e.at }))
        .collect();
    LogState {
        history,
        lines: vault::open_lines(&vault::load(&log_path(&app))),
    }
}

#[tauri::command]
async fn append_log(app: AppHandle, message: String) {
    vault_push(&app, &message);
}

#[tauri::command]
async fn clear_log(app: AppHandle) {
    vault_clear(&app);
}

#[tauri::command]
async fn get_exit_ip(app: AppHandle) -> String {
    refresh_ip(&app).await
}

fn show_main(app: &AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        let build = handle.state::<WindowBuildLock>();
        let _guard = build.0.lock();
        let mut mains: Vec<_> = handle
            .webview_windows()
            .into_values()
            .filter(|w| w.label() == "main")
            .collect();
        if mains.len() > 1 {
            mains.sort_by_key(|w| std::cmp::Reverse(w.is_visible().unwrap_or(false)));
            for extra in mains.drain(1..) {
                let _ = extra.destroy();
            }
        }
        if let Some(window) = mains.into_iter().next() {
            let _ = window.show();
            let _ = window.set_focus();
            return;
        }
        let built = handle.config().app.windows.first().cloned().map(|config| {
            tauri::WebviewWindowBuilder::from_config(&handle, &config)
                .and_then(|builder| builder.build())
        });
        match built {
            Some(Ok(window)) => {
                let _ = window.show();
                let _ = window.set_focus();
            }
            _ => {
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                } else {
                    notify_no_window(&handle);
                }
            }
        }
    });
}

fn notify_no_window(app: &AppHandle) {
    if !notifications_allowed(app) {
        return;
    }
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title("Warper")
        .body("Window could not open — Warper is running in the tray.")
        .show();
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};

    let Some(icon) = app.default_window_icon().cloned() else {
        return Ok(());
    };
    let show = MenuItem::with_id(app, "show", "Show Warper", true, None::<&str>)?;
    let quick = MenuItem::with_id(app, "quick", "Quick Reset", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quick, &quit])?;

    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Warper")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quick" => {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let guard = handle
                        .state::<WarpLock>()
                        .0
                        .clone()
                        .try_lock_owned()
                        .map_err(|_| "another operation is already running".to_string());
                    let Ok(_guard) = guard else {
                        emit(&handle, "quick reset ignored — already running");
                        return;
                    };
                    let mode = warp::get_mode()
                        .await
                        .unwrap_or_else(|_| "warp".to_string());
                    match run_quick_reset(&handle, &mode).await {
                        Ok(_) => {
                            emit(&handle, "quick reset complete");
                            emit_ip(&handle).await;
                        }
                        Err(e) => emit(&handle, &format!("quick reset failed: {e}")),
                    }
                    note_op(&handle);
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let autostart = tauri_plugin_autostart::Builder::new()
        .app_name("Warper")
        .args(["--minimized"])
        .build();
    let from_autostart = std::env::args().any(|a| a == "--minimized");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .plugin(autostart)
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(WarpLock(Arc::new(tokio::sync::Mutex::new(()))))
        .manage(AutoState(Mutex::new(Instant::now())))
        .manage(WindowBuildLock(Mutex::new(())))
        .manage(VaultWarned(Mutex::new(false)))
        .setup(move |app| {
            aumid::ensure_aumid(app.handle());
            let _ = build_tray(app.handle());
            migrate_history_once(app.handle());
            spawn_status_watcher(app.handle().clone());
            spawn_auto_reset(app.handle().clone());
            spawn_ip_watch(app.handle().clone());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let start_min = start_minimized(&handle).await;
                let minimized = from_autostart && start_min;
                if let Some(note) = repair_autostart_if_stale(&handle) {
                    emit(&handle, &note);
                }
                let autostart = handle.autolaunch().is_enabled().unwrap_or(false);
                emit(
                    &handle,
                    &format!("boot: autostart={autostart} from_autostart={from_autostart} start_minimized={start_min}"),
                );
                if !minimized {
                    show_main(&handle);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.destroy();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_identity,
            get_warp_info,
            get_settings_path,
            get_mode,
            get_autostart,
            set_autostart,
            warp_connect,
            warp_standby,
            quick_reset,
            full_reset,
            auto_countdown,
            reset_auto_timer,
            get_log_state,
            append_log,
            clear_log,
            get_exit_ip
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            let _ = app;
        });
}

#[cfg(windows)]
fn run_command() -> Option<String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };

    let mut key = HKEY::default();
    if unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run"),
            None,
            KEY_READ,
            &mut key,
        )
    } != ERROR_SUCCESS
    {
        return None;
    }
    let mut size = 0u32;
    let sized = unsafe {
        RegQueryValueExW(
            key,
            &HSTRING::from("Warper"),
            None,
            None,
            None,
            Some(&mut size),
        )
    } == ERROR_SUCCESS
        && size > 0;
    let mut buf = vec![0u8; size as usize];
    let ok = sized
        && unsafe {
            RegQueryValueExW(
                key,
                &HSTRING::from("Warper"),
                None,
                None,
                Some(buf.as_mut_ptr()),
                Some(&mut size),
            )
        } == ERROR_SUCCESS;
    unsafe {
        let _ = RegCloseKey(key);
    }
    if !ok {
        return None;
    }
    let (chunks, _) = buf.as_chunks::<2>();
    let wide: Vec<u16> = chunks
        .iter()
        .take(size as usize / 2)
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    Some(
        String::from_utf16_lossy(&wide)
            .trim_matches('\0')
            .trim()
            .to_string(),
    )
}

fn repair_autostart_if_stale(app: &AppHandle) -> Option<String> {
    if !app.autolaunch().is_enabled().unwrap_or(false) {
        return None;
    }
    let current = std::env::current_exe()
        .ok()?
        .to_string_lossy()
        .to_lowercase();
    let stored = run_command()?;
    if stored.to_lowercase().contains(&current) {
        return None;
    }
    app.autolaunch().enable().ok()?;
    Some(format!("autostart entry repaired (was: {stored})"))
}

async fn start_minimized(app: &AppHandle) -> bool {
    use tauri_plugin_store::StoreExt;
    let (path, _) = settings_path(app);
    let Ok(store) = app.store(path) else {
        return true;
    };
    store
        .get("startMinimized")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn spawn_auto_reset(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            if !auto_due(&app).await {
                continue;
            }
            let guard = app
                .state::<WarpLock>()
                .0
                .clone()
                .try_lock_owned()
                .map_err(|_| "another operation is already running".to_string());
            let Ok(_guard) = guard else {
                continue;
            };
            let stored: Option<String> = {
                use tauri_plugin_store::StoreExt;
                let (path, _) = settings_path(&app);
                app.store(path)
                    .ok()
                    .and_then(|store| store.get("connectMode"))
                    .and_then(|v| v.as_str().map(str::to_string))
            };
            let mode = match stored {
                Some(mode) => mode,
                None => warp::get_mode()
                    .await
                    .unwrap_or_else(|_| "warp".to_string()),
            };
            if run_quick_reset(&app, &mode).await.is_ok() {
                emit_ip(&app).await;
            }
            note_op(&app);
        }
    });
}

async fn auto_due(app: &AppHandle) -> bool {
    use tauri_plugin_store::StoreExt;
    let (path, _) = settings_path(app);
    let Ok(store) = app.store(path) else {
        return false;
    };
    let enabled = store
        .get("autoReset")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !enabled {
        return false;
    }
    let mins = store
        .get("intervalMinutes")
        .and_then(|v| v.as_u64())
        .unwrap_or(15);
    let elapsed = app
        .state::<AutoState>()
        .0
        .lock()
        .map(|last| last.elapsed())
        .unwrap_or(Duration::ZERO);
    elapsed >= Duration::from_secs(mins.saturating_mul(60))
}

fn spawn_status_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut failed = false;
        let mut missing_notified = false;
        loop {
            if !warp::warp_info().await.installed {
                if !missing_notified {
                    let _ = app.emit("warp-missing", ());
                    missing_notified = true;
                }
                tokio::time::sleep(Duration::from_secs(15)).await;
                continue;
            }
            missing_notified = false;
            if let Err(error) = run_watch(&app).await {
                if !failed {
                    emit(&app, &format!("status stream lost ({error}), retrying…"));
                    failed = true;
                }
            } else {
                failed = false;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn run_watch(app: &AppHandle) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = tokio::process::Command::new(warp::warp_cli_path())
        .creation_flags(0x0800_0000)
        .args(["--json", "--listen", "status"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to launch warp-cli listener: {e}"))?;

    if let Err(error) = bind_child_lifetime(&child) {
        emit(app, &format!("warning: listener not job-bound ({error})"));
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "no stdout from warp-cli listener".to_string())?;
    let mut lines = BufReader::new(stdout).lines();
    let mut last: Option<String> = None;

    loop {
        tokio::select! {
            next = lines.next_line() => {
                let line = next.map_err(|e| format!("listener read error: {e}"))?;
                let Some(line) = line else { break };
                let text = line.trim();
                if text.is_empty() || Some(text) == last.as_deref() {
                    continue;
                }
                if let Ok(status) = warp::parse_status(text) {
                    last = Some(text.to_string());
                    let _ = app.emit("warp-status", status);
                }
            }
            _ = child.wait() => break,
        }
    }
    Err("warp-cli listener exited".to_string())
}

fn bind_child_lifetime(child: &tokio::process::Child) -> Result<(), String> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    unsafe {
        let job =
            std::mem::ManuallyDrop::new(CreateJobObjectW(None, None).map_err(|e| e.to_string())?);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            *job,
            JobObjectExtendedLimitInformation,
            &raw const limits as *const _,
            std::mem::size_of_val(&limits) as u32,
        )
        .map_err(|e| e.to_string())?;
        let raw = child
            .raw_handle()
            .ok_or_else(|| "listener has no process handle".to_string())?;
        AssignProcessToJobObject(*job, HANDLE(raw as _)).map_err(|e| e.to_string())?;
    }
    Ok(())
}
