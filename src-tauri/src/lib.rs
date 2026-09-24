mod warp;

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use warp::{WarpIdentity, WarpInfo, WarpStatus};

pub(crate) struct WarpLock(pub(crate) Arc<tokio::sync::Mutex<()>>);

/// Settings file name, shared by the store path and the cleanup script.
pub(crate) const SETTINGS_FILE: &str = "warper-settings.json";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsLocation {
    path: String,
    portable: bool,
    imported: bool,
}

/// Portable exe folder when writable, else the app-data dir.
pub(crate) fn settings_path(app: &AppHandle) -> (std::path::PathBuf, bool, bool) {
    let mut imported = false;
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let probe = dir.join(".warper-write-test");
            if std::fs::write(&probe, b"").is_ok() {
                let _ = std::fs::remove_file(&probe);
                let candidate = dir.join(SETTINGS_FILE);
                if !candidate.exists() {
                    if let Ok(legacy) = app.path().app_data_dir().map(|d| d.join("settings.json")) {
                        if legacy.exists() && std::fs::copy(&legacy, &candidate).is_ok() {
                            imported = true;
                        }
                    }
                }
                return (candidate, true, imported);
            }
        }
    }
    let fallback = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(SETTINGS_FILE);
    (fallback, false, imported)
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
}

/// Push the fresh exit IP so the frontend can toast on change.
/// The frontend owns dedupe, history, the settings gate, and OS permission.
async fn emit_ip(app: &AppHandle) {
    let ip = warp::fetch_exit_ip().await;
    let _ = app.emit("warp-ip", ip);
}

async fn locked(state: State<'_, WarpLock>) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    warp::require_admin(warp::is_elevated().await)?;
    state
        .0
        .clone()
        .try_lock_owned()
        .map_err(|_| "another operation is already running".to_string())
}

/// `registration new` refuses while a stale registration exists, so clear it and retry once.
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
    let (path, portable, imported) = settings_path(&app);
    SettingsLocation {
        path: path.to_string_lossy().into_owned(),
        portable,
        imported,
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
        Ok("WARP disconnected".to_string())
    } else {
        emit(&app, &format!("switching to {standby} mode…"));
        warp::set_mode(standby).await?;
        emit(&app, &format!("standby mode ({standby})"));
        emit_ip(&app).await;
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
    result
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
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
                    // No frontend settings here; reuse the live mode.
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
    // `--minimized` is honored only when the stored `startMinimized` setting agrees.
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
        .setup(move |app| {
            let _ = build_tray(app.handle());
            spawn_status_watcher(app.handle().clone());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Window starts hidden to avoid a flash when tray-starting.
                let minimized = from_autostart && start_minimized(&handle).await;
                if !minimized {
                    show_main(&handle);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
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
            full_reset
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Falls back to minimized when the store is unreadable during autostart.
async fn start_minimized(app: &AppHandle) -> bool {
    use tauri_plugin_store::StoreExt;
    let (path, _, _) = settings_path(app);
    let Ok(store) = app.store(path) else {
        return true;
    };
    store
        .get("startMinimized")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Push `warp-status` events on change; slow-poll while WARP is missing.
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
    // Kill the child with Warper even on taskkill/crash, so no
    // orphan holds handles on the portable folder.
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

/// Job Object with KILL_ON_JOB_CLOSE, leaked so the kill fires at process teardown.
fn bind_child_lifetime(child: &tokio::process::Child) -> Result<(), String> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    unsafe {
        // Leaked on purpose: closing the job would lift KILL_ON_JOB_CLOSE.
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
