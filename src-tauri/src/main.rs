// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod net;
mod usage;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};

use net::{Clients, Reachability, Throughput};

/// What the tray + flyout render. camelCase for the JS side.
#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct StatusPayload {
    state: String,            // "Fast", "Online", "Slow", "Sign-in needed", "Offline", "Checking"
    tier: String,             // machine tier: fast|normal|slow|idle|portal|offline|checking
    color: String,            // green|amber|red|slate — also picks the tray icon
    latency_ms: Option<f64>,
    down_mbps: f64,           // live passive rate (drives the globe spin)
    up_mbps: f64,
    test_mbps: Option<f64>,   // last on-demand speed test, if recent
}

impl StatusPayload {
    fn checking() -> Self {
        StatusPayload {
            state: "Checking".into(),
            tier: "checking".into(),
            color: "slate".into(),
            ..Default::default()
        }
    }

    fn tooltip(&self) -> String {
        let mut s = format!("NetCheck — {}", self.state);
        if let Some(ms) = self.latency_ms {
            s.push_str(&format!(" · {} ms", ms.round() as i64));
        }
        if self.down_mbps >= 0.1 {
            s.push_str(&format!(" · {:.1} Mbps", self.down_mbps));
        }
        s
    }
}

#[derive(Default)]
struct Monitor {
    latest: StatusPayload,
    last_test_mbps: Option<f64>,
    last_test_at: Option<Instant>,
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Map a probe result + live rate (+ any recent speed test) to a status.
/// Reachability problems always win. When reachable, the colour tier comes from a
/// recent speed test if we have one (<5 min); otherwise we report a healthy "Online"
/// rather than misreading idle passive traffic as "slow".
fn classify(
    reach: Reachability,
    latency: Option<f64>,
    tp: Throughput,
    recent_test: Option<f64>,
) -> StatusPayload {
    let (state, tier, color) = match reach {
        Reachability::NoInternet => ("Offline", "offline", "red"),
        Reachability::CaptivePortal => ("Sign-in needed", "portal", "amber"),
        Reachability::Online => match recent_test {
            Some(mbps) if mbps >= 200.0 => ("Fast", "fast", "green"),
            Some(mbps) if mbps >= 50.0 => ("Online", "normal", "green"),
            Some(_) => ("Slow", "slow", "amber"),
            None => ("Online", "normal", "green"),
        },
    };
    StatusPayload {
        state: state.into(),
        tier: tier.into(),
        color: color.into(),
        latency_ms: latency.map(|m| (m * 10.0).round() / 10.0),
        down_mbps: round1(tp.down_mbps),
        up_mbps: round1(tp.up_mbps),
        test_mbps: recent_test.map(round1),
    }
}

fn tray_icon_for(color: &str) -> Image<'static> {
    match color {
        "red" => tauri::include_image!("icons/tray-red.png"),
        "amber" => tauri::include_image!("icons/tray-amber.png"),
        "slate" => tauri::include_image!("icons/tray-slate.png"),
        _ => tauri::include_image!("icons/tray-green.png"),
    }
}

/// Show/hide the flyout, anchored to the tray.
fn toggle_flyout(app: &AppHandle) {
    use tauri_plugin_positioner::{Position, WindowExt};
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.move_window(Position::TrayBottomRight);
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}

/// Background poll: reachability + latency + live throughput, every few seconds.
/// Updates the tray colour/tooltip and emits a `status` event to the flyout.
async fn monitor_loop(app: AppHandle) {
    let probe = app.state::<Clients>().inner().probe.clone();

    loop {
        let reach = net::check_reachability(&probe).await;
        let latency = net::measure_latency_ms("1.1.1.1:443").await;
        let tp = net::sample_throughput(Duration::from_millis(1000)).await;

        let recent_test = {
            let m = app.state::<Mutex<Monitor>>();
            let g = m.lock().unwrap();
            match (g.last_test_mbps, g.last_test_at) {
                (Some(v), Some(t)) if t.elapsed() < Duration::from_secs(300) => Some(v),
                _ => None,
            }
        };

        let payload = classify(reach, latency, tp, recent_test);

        if let Some(tray) = app.tray_by_id("main-tray") {
            let _ = tray.set_icon(Some(tray_icon_for(&payload.color)));
            let _ = tray.set_tooltip(Some(payload.tooltip()));
        }

        {
            let m = app.state::<Mutex<Monitor>>();
            m.lock().unwrap().latest = payload.clone();
        }
        let _ = app.emit("status", &payload);

        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

#[tauri::command]
fn get_current_status(monitor: State<'_, Mutex<Monitor>>) -> StatusPayload {
    monitor.lock().unwrap().latest.clone()
}

#[tauri::command]
async fn run_speed_test(
    clients: State<'_, Clients>,
    monitor: State<'_, Mutex<Monitor>>,
) -> Result<f64, String> {
    let mbps = net::run_speed_test(&clients.speed, 25_000_000).await?;
    let rounded = round1(mbps);
    {
        let mut m = monitor.lock().unwrap();
        m.last_test_mbps = Some(rounded);
        m.last_test_at = Some(Instant::now());
    }
    Ok(rounded)
}

#[tauri::command]
async fn get_data_usage(period: String) -> Result<usage::DataUsage, String> {
    // WinRT GetNetworkUsageAsync blocks (.get()), so run it off the async runtime.
    tauri::async_runtime::spawn_blocking(move || usage::data_usage(&period))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn set_autostart(app: AppHandle, enable: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    if enable { mgr.enable() } else { mgr.disable() }.map_err(|e| e.to_string())
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

fn main() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        // single-instance must be registered first.
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }))
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                None::<Vec<&str>>,
            ));
    }

    builder
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_current_status,
            run_speed_test,
            get_data_usage,
            set_autostart,
            get_autostart,
            open_external
        ])
        .setup(|app| {
            app.manage(Clients::new());
            app.manage(Mutex::new(Monitor {
                latest: StatusPayload::checking(),
                ..Default::default()
            }));

            // Tray with a context menu; left-click opens the flyout.
            let show = MenuItemBuilder::with_id("show", "Show NetCheck").build(app)?;
            let test = MenuItemBuilder::with_id("test", "Run speed test").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit NetCheck").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&show)
                .item(&test)
                .separator()
                .item(&quit)
                .build()?;

            TrayIconBuilder::with_id("main-tray")
                .icon(tray_icon_for("slate"))
                .tooltip("NetCheck — checking…")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => toggle_flyout(app),
                    "test" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = w.emit("run-speed-test", ());
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_flyout(tray.app_handle());
                    }
                })
                .build(app)?;

            // Hide the flyout when it loses focus (menu-bar dismiss behaviour).
            if let Some(win) = app.get_webview_window("main") {
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let _ = w.hide();
                    }
                });
            }

            tauri::async_runtime::spawn(monitor_loop(app.handle().clone()));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running NetCheck");
}
