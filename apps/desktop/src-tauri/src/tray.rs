//! The tray icon (Windows, Linux) and menu bar icon (macOS), and its panel (tray spec
//! `docs/plan/2026-11-17-tray.md`).
//!
//! - The icon lives in the window's process. The agent starts that process with `--tray` (no
//!   window) at its own start when the setting `tray-icon` is on. While the icon is shown,
//!   closing the window does not end the process.
//! - Icon state, tooltip and the Linux menu come from Rust ([`tray_view`]), from one agent
//!   subscription that never starts the agent. A panel need not be open for them to be right.
//! - Windows and macOS: a click opens the dark panel (a borderless webview, `#panel`), made on
//!   click and closed when it loses focus. Linux (AppIndicator reports no clicks): a native
//!   menu with the same actions.
//! - Actions that outlive the panel (pick and copy, open the window, quit) run here, not in
//!   the page.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl};
use uc_agent::ipc::Client;

pub const TRAY_ID: &str = "uc";
pub const PANEL: &str = "panel";
/// The panel's width in CSS pixels, with the clear edge that holds its shadow.
pub const PANEL_W: f64 = 320.0;
pub const PANEL_EDGE: f64 = 14.0;

/// The icon picture: icon A with a badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Idle,
    Busy,
    Paused,
    Attention,
}

/// What the icon, its tooltip and the Linux menu show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayView {
    pub look: Look,
    /// The state word: Synced, Receiving, Sending, Paused, Not connected, ...
    pub word: String,
    /// Percent of the file copy that moves now.
    pub pct: Option<u8>,
    pub paused: bool,
    pub moving: bool,
}

impl TrayView {
    pub fn tooltip(&self) -> String {
        match self.pct {
            Some(p) => format!("Universal Clipboard: {} {p} %", self.word),
            None => format!("Universal Clipboard: {}", self.word),
        }
    }
}

/// The view for an agent `status` reply, or none when the agent does not answer.
pub fn tray_view(status: Option<&Value>) -> TrayView {
    let Some(s) = status else {
        return TrayView {
            look: Look::Attention,
            word: "Not running".into(),
            pct: None,
            paused: false,
            moving: false,
        };
    };
    let state = s["state"].as_str().unwrap_or("");
    let paused = s["paused"].as_bool().unwrap_or(false);
    let t = &s["transfer"];
    let phase = t["phase"].as_str().unwrap_or("idle");
    let moving = matches!(phase, "sending" | "receiving" | "preparing");
    let pct = moving.then(|| {
        let (done, total) = (
            t["done"].as_u64().unwrap_or(0),
            t["total"].as_u64().unwrap_or(0),
        );
        ((done.min(total) * 100).checked_div(total).unwrap_or(0)) as u8
    });
    let (look, word) = match state {
        "needs-attention" => (
            Look::Attention,
            match s["fix_code"].as_str() {
                Some("vpn" | "no-lan" | "firewall") => "Not connected",
                _ => "Needs attention",
            },
        ),
        "paused" => (Look::Paused, "Paused"),
        "not-set-up" => (Look::Idle, "Not set up"),
        _ if moving => (
            Look::Busy,
            if phase == "receiving" {
                "Receiving"
            } else {
                "Sending"
            },
        ),
        "no-peers" => (Look::Idle, "Not connected"),
        _ => (Look::Idle, "Synced"),
    };
    TrayView {
        look,
        word: word.into(),
        pct: if look == Look::Busy { pct } else { None },
        paused,
        moving,
    }
}

/// A rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x + self.w
    }
    fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

/// Where the panel (its window, clear edge included) goes, next to the icon and inside the
/// work area of the icon's screen. `edge`: the clear edge in physical pixels, so the visible
/// panel sits `gap` away from the taskbar or menu bar. Above or below the icon, the visible
/// panel's right edge lines up with the icon's right edge, like the system tray menus (Max
/// 2026-11-18).
pub fn panel_position(icon: Rect, work: Rect, size: (i32, i32), edge: i32, gap: i32) -> (i32, i32) {
    let (w, h) = size;
    let cy = icon.y + icon.h / 2;
    let right_x = icon.right() - w + edge;
    let centred_y = cy - h / 2;
    let (x, y) = if icon.y >= work.bottom() - 1 {
        // Taskbar at the bottom: above it.
        (right_x, work.bottom() - h + edge - gap)
    } else if icon.bottom() <= work.y + 1 {
        // Taskbar or menu bar at the top: below it.
        (right_x, work.y - edge + gap)
    } else if icon.x >= work.right() - 1 {
        // Taskbar on the right.
        (work.right() - w + edge - gap, centred_y)
    } else if icon.right() <= work.x + 1 {
        // Taskbar on the left.
        (work.x - edge + gap, centred_y)
    } else if cy > work.y + work.h / 2 {
        // The icon is inside the work area (the Windows overflow flyout): above it.
        (right_x, icon.y - h + edge - gap)
    } else {
        (right_x, icon.bottom() - edge + gap)
    };
    // Keep the visible panel inside the work area.
    let x = x.clamp(work.x - edge, (work.right() - w + edge).max(work.x - edge));
    let y = y.clamp(work.y - edge, (work.bottom() - h + edge).max(work.y - edge));
    (x, y)
}

/// The arguments the app was started with: `--tray` starts it with the icon and no window.
pub fn tray_arg(args: &[String]) -> bool {
    args.iter().any(|a| a == "--tray")
}

// ------------------------------------------------------------------ icons

macro_rules! png {
    ($($name:tt)+) => {
        include_bytes!(concat!("../icons/tray/", $($name)+, ".png")) as &[u8]
    };
}

#[cfg(windows)]
fn look_name(l: Look) -> &'static str {
    match l {
        Look::Idle => "idle",
        Look::Busy => "busy",
        Look::Paused => "paused",
        Look::Attention => "attention",
    }
}

/// The PNG for a look, in the colour and size this system wants.
fn icon_png(look: Look, scale: f64) -> &'static [u8] {
    let _ = scale;
    #[cfg(target_os = "macos")]
    {
        match look {
            Look::Idle => png!("idle-template-36"),
            Look::Busy => png!("busy-template-36"),
            Look::Paused => png!("paused-template-36"),
            Look::Attention => png!("attention-template-36"),
        }
    }
    #[cfg(target_os = "windows")]
    {
        let light_bar = uc_agent::win::taskbar_is_light();
        let px = if scale >= 1.75 {
            32
        } else if scale >= 1.375 {
            24
        } else if scale >= 1.125 {
            20
        } else {
            16
        };
        win_png(look_name(look), light_bar, px)
    }
    #[cfg(target_os = "linux")]
    {
        match look {
            Look::Idle => png!("idle-light-32"),
            Look::Busy => png!("busy-light-32"),
            Look::Paused => png!("paused-light-32"),
            Look::Attention => png!("attention-light-32"),
        }
    }
}

#[cfg(target_os = "windows")]
fn win_png(look: &str, light_bar: bool, px: u32) -> &'static [u8] {
    macro_rules! table {
        ($($l:literal),*) => {
            match (look, light_bar, px) {
                $(
                    ($l, false, 16) => png!(concat!($l, "-light-16")),
                    ($l, false, 20) => png!(concat!($l, "-light-20")),
                    ($l, false, 24) => png!(concat!($l, "-light-24")),
                    ($l, false, 32) => png!(concat!($l, "-light-32")),
                    ($l, true, 16) => png!(concat!($l, "-dark-16")),
                    ($l, true, 20) => png!(concat!($l, "-dark-20")),
                    ($l, true, 24) => png!(concat!($l, "-dark-24")),
                    ($l, true, 32) => png!(concat!($l, "-dark-32")),
                )*
                _ => png!("idle-light-32"),
            }
        };
    }
    table!("idle", "busy", "paused", "attention")
}

// ------------------------------------------------------------------ runtime state

/// Shared tray state, managed by the app.
#[derive(Default)]
pub struct Tray {
    /// The icon is shown (so closing the last window does not end the process).
    pub shown: AtomicBool,
    /// The last view, to skip work when nothing changed.
    view: Mutex<Option<TrayView>>,
    /// The last status reply, so transfer events update it without a new call.
    status: Mutex<Option<Value>>,
    /// The last icon rectangle (physical), for placing the panel.
    icon_rect: Mutex<Option<Rect>>,
    /// When the icon was clicked, to measure how fast the panel shows (spec 3.2: 150 ms).
    clicked: Mutex<Option<Instant>>,
    /// When the panel last closed: a click on the icon that closed it by taking the focus
    /// must not open it again.
    last_close: Mutex<Option<Instant>>,
    /// The panel is on screen. Until then a lost focus is ignored: on Windows the new, still
    /// hidden window gets the focus and loses it at once, which closed it before it showed.
    panel_up: AtomicBool,
    /// The screen the next window should open on (read by the page via `launch_route`).
    pub pending_route: Mutex<Option<String>>,
    #[cfg(target_os = "linux")]
    menu: Mutex<Option<LinuxMenu>>,
}

fn tray(app: &AppHandle) -> tauri::State<'_, Tray> {
    app.state::<Tray>()
}

/// Show the icon (once), or update it.
pub fn show(app: &AppHandle) {
    let t = tray(app);
    if t.shown.swap(true, Ordering::SeqCst) {
        return;
    }
    let view = t
        .view
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| tray_view(None));
    let scale = primary_scale(app);
    let Ok(img) = Image::from_bytes(icon_png(view.look, scale)) else {
        return;
    };
    #[allow(unused_mut)]
    let mut b = TrayIconBuilder::with_id(TRAY_ID)
        .icon(img)
        .tooltip(view.tooltip())
        .icon_as_template(cfg!(target_os = "macos"))
        .show_menu_on_left_click(false);
    #[cfg(target_os = "linux")]
    {
        match LinuxMenu::new(app) {
            Ok(m) => {
                b = b.menu(&m.menu).show_menu_on_left_click(true);
                m.update(&view);
                *t.menu.lock().unwrap() = Some(m);
            }
            Err(e) => eprintln!("tray menu: {e}"),
        }
        b = b.on_menu_event(|app, e| on_menu(app, e.id().as_ref()));
    }
    let b = b.on_tray_icon_event(|icon, e| on_icon_event(icon.app_handle(), e));
    if let Err(e) = b.build(app) {
        eprintln!("tray icon: {e}");
        t.shown.store(false, Ordering::SeqCst);
    }
}

/// Remove the icon. With no window left, the process ends.
pub fn hide(app: &AppHandle) {
    let t = tray(app);
    if !t.shown.swap(false, Ordering::SeqCst) {
        return;
    }
    close_panel(app);
    let _ = app.remove_tray_by_id(TRAY_ID);
    if app.webview_windows().is_empty() {
        app.exit(0);
    }
}

/// Set the view (icon, tooltip, Linux menu) from a status reply.
pub fn set_status(app: &AppHandle, status: Option<&Value>) {
    let view = tray_view(status);
    let t = tray(app);
    *t.status.lock().unwrap() = status.cloned();
    let mut last = t.view.lock().unwrap();
    if last.as_ref() == Some(&view) {
        return;
    }
    let look_changed = last.as_ref().map(|v| v.look) != Some(view.look);
    *last = Some(view.clone());
    drop(last);
    if let Some(icon) = app.tray_by_id(TRAY_ID) {
        if look_changed && let Ok(img) = Image::from_bytes(icon_png(view.look, primary_scale(app)))
        {
            let _ = icon.set_icon(Some(img));
            #[cfg(target_os = "macos")]
            let _ = icon.set_icon_as_template(true);
        }
        let _ = icon.set_tooltip(Some(view.tooltip()));
    }
    #[cfg(target_os = "linux")]
    if let Some(m) = t.menu.lock().unwrap().as_ref() {
        m.update(&view);
    }
}

/// A `transfer` event (up to 4 a second): update the last status with it.
pub fn set_transfer(app: &AppHandle, transfer: &Value) {
    let Some(mut s) = tray(app).status.lock().unwrap().clone() else {
        return;
    };
    s["transfer"] = if transfer["phase"] == "idle" {
        Value::Null
    } else {
        transfer.clone()
    };
    set_status(app, Some(&s));
}

fn primary_scale(app: &AppHandle) -> f64 {
    app.primary_monitor()
        .ok()
        .flatten()
        .map_or(1.0, |m| m.scale_factor())
}

// ------------------------------------------------------------------ panel

fn on_icon_event(app: &AppHandle, e: TrayIconEvent) {
    let TrayIconEvent::Click {
        rect,
        button,
        button_state: MouseButtonState::Up,
        position,
        ..
    } = e
    else {
        return;
    };
    if !matches!(button, MouseButton::Left | MouseButton::Right) {
        return;
    }
    let scale = app
        .monitor_from_point(position.x, position.y)
        .ok()
        .flatten()
        .map_or(1.0, |m| m.scale_factor());
    let p: PhysicalPosition<i32> = rect.position.to_physical(scale);
    let s: PhysicalSize<u32> = rect.size.to_physical(scale);
    let r = Rect {
        x: p.x,
        y: p.y,
        w: s.width as i32,
        h: s.height as i32,
    };
    *tray(app).icon_rect.lock().unwrap() = Some(r);
    *tray(app).clicked.lock().unwrap() = Some(Instant::now());
    // Not in this event handler: building a webview here deadlocks on Windows.
    let app = app.clone();
    tauri::async_runtime::spawn(async move { toggle_panel(&app) });
}

fn toggle_panel(app: &AppHandle) {
    if app.get_webview_window(PANEL).is_some() {
        close_panel(app);
        return;
    }
    // This click took the focus from the panel and so closed it: leave it closed.
    if tray(app)
        .last_close
        .lock()
        .unwrap()
        .is_some_and(|t| t.elapsed() < Duration::from_millis(300))
    {
        return;
    }
    let b =
        tauri::WebviewWindowBuilder::new(app, PANEL, WebviewUrl::App("index.html#panel".into()))
            .title("Universal Clipboard")
            .inner_size(PANEL_W + 2.0 * PANEL_EDGE, 420.0)
            .decorations(false)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .visible(false)
            .focused(true);
    #[cfg(any(windows, target_os = "macos"))]
    let b = b.transparent(true);
    tray(app).panel_up.store(false, Ordering::SeqCst);
    match b.build() {
        Ok(w) => {
            let app2 = app.clone();
            w.on_window_event(move |e| {
                if let tauri::WindowEvent::Focused(false) = e
                    && tray(&app2).panel_up.load(Ordering::SeqCst)
                {
                    close_panel(&app2);
                }
            });
        }
        Err(e) => eprintln!("panel: {e}"),
    }
}

/// Close the panel if it is open.
pub fn close_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(PANEL) {
        *tray(app).last_close.lock().unwrap() = Some(Instant::now());
        tray(app).panel_up.store(false, Ordering::SeqCst);
        let _ = w.destroy();
    }
}

/// The page measured itself (CSS pixels, clear edge included): size, place and show it.
#[tauri::command]
pub fn panel_ready(app: AppHandle, height: f64) {
    let Some(w) = app.get_webview_window(PANEL) else {
        return;
    };
    let icon = *tray(&app).icon_rect.lock().unwrap();
    let mon = icon
        .and_then(|r| {
            app.monitor_from_point(f64::from(r.x), f64::from(r.y))
                .ok()
                .flatten()
        })
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = mon else {
        let _ = w.show();
        let _ = w.set_focus();
        tray(&app).panel_up.store(true, Ordering::SeqCst);
        return;
    };
    let scale = m.scale_factor();
    let size = (
        ((PANEL_W + 2.0 * PANEL_EDGE) * scale).round() as i32,
        (height.clamp(120.0, 900.0) * scale).round() as i32,
    );
    let wa = m.work_area();
    let work = Rect {
        x: wa.position.x,
        y: wa.position.y,
        w: wa.size.width as i32,
        h: wa.size.height as i32,
    };
    let icon = icon.unwrap_or(Rect {
        x: work.right() - 40,
        y: work.bottom(),
        w: 32,
        h: 32,
    });
    let edge = (PANEL_EDGE * scale).round() as i32;
    let gap = (8.0 * scale).round() as i32;
    let (x, y) = panel_position(icon, work, size, edge, gap);
    // macOS: like the system menus. macOS keeps a window from going under the menu bar, so
    // the page drops its clear top edge (html.mac) and the window starts just below the bar;
    // and panel_position already puts the panel's right edge under the icon's right edge.
    #[cfg(target_os = "macos")]
    let y = {
        let _ = (y, gap);
        work.y + (3.0 * scale).round() as i32
    };
    let _ = w.set_size(PhysicalSize::new(size.0 as u32, size.1 as u32));
    let _ = w.set_position(PhysicalPosition::new(x, y));
    let _ = w.show();
    let _ = w.set_focus();
    tray(&app).panel_up.store(true, Ordering::SeqCst);
    #[cfg(target_os = "macos")]
    activate_app(&app);
    if let Some(t) = tray(&app).clicked.lock().unwrap().take() {
        log_open_time(t.elapsed());
    }
}

/// Click-to-shown times of the panel, in `<temp>/uc-panel-open.log` (the last 50), so the
/// 150 ms target can be checked on a real machine. No content, only numbers.
/// macOS: make the app active, not only the panel's window key. A status-item click does not
/// activate the app, and set_focus uses activateIgnoringOtherApps, which macOS 14+ may ignore.
/// Without an active app WebKit gets no mouse moves: no hover, no tooltips.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn activate_app(app: &AppHandle) {
    let _ = app.run_on_main_thread(|| {
        use objc2::runtime::{AnyClass, AnyObject};
        use objc2::{msg_send, sel};
        let Some(cls) = AnyClass::get(c"NSApplication") else {
            return;
        };
        // SAFETY: plain AppKit calls on the main thread; `activate` is checked before use.
        unsafe {
            let ns_app: *mut AnyObject = msg_send![cls, sharedApplication];
            let Some(ns_app) = ns_app.as_ref() else {
                return;
            };
            let has_activate: bool = msg_send![ns_app, respondsToSelector: sel!(activate)];
            if has_activate {
                let () = msg_send![ns_app, activate];
            } else {
                let () = msg_send![ns_app, activateIgnoringOtherApps: true];
            }
        }
    });
}

fn log_open_time(d: Duration) {
    let f = std::env::temp_dir().join("uc-panel-open.log");
    let mut lines: Vec<String> = std::fs::read_to_string(&f)
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect();
    lines.push(d.as_millis().to_string());
    let from = lines.len().saturating_sub(50);
    let _ = std::fs::write(&f, lines[from..].join("\n") + "\n");
}

/// Copy files... / Copy folder... from the panel or the Linux menu: close the panel, open the
/// system picker, then make the picked items the latest copy.
#[tauri::command]
pub async fn tray_copy(app: AppHandle, folder: bool) -> Result<(), crate::Failure> {
    close_panel(&app);
    let paths = crate::pick(&app, folder, false).await;
    if paths.is_empty() {
        return Ok(());
    }
    crate::agent_call("files.copy".into(), json!({"paths": paths, "picked": true}))
        .await
        .map(|_| ())
}

/// Open the window, on a screen: `settings`, `vpn` or none (Home).
#[tauri::command]
pub async fn tray_open(app: AppHandle, route: Option<String>) {
    close_panel(&app);
    open_window(&app, route);
}

/// Quit: stop the agent (without starting one) and end this process. Sync stays off until the
/// next login or until the app is opened again.
#[tauri::command]
pub async fn tray_quit(app: AppHandle) {
    close_panel(&app);
    if let Ok(p) = crate::profile()
        && let Ok(mut c) = Client::connect(&p).await
    {
        let _ = tokio::time::timeout(Duration::from_secs(3), c.call("quit", json!({}))).await;
    }
    app.exit(0);
}

/// macOS (Max 2026-11-17): a Dock icon only while the window is open. Without a window the
/// app lives in the menu bar alone, and the user can hide that icon in System Settings >
/// Menu Bar like any other.
#[cfg(target_os = "macos")]
pub fn dock_icon(app: &AppHandle, on: bool) {
    let _ = app.set_activation_policy(if on {
        tauri::ActivationPolicy::Regular
    } else {
        tauri::ActivationPolicy::Accessory
    });
}

/// Create or show the main window. A new window reads the route through `launch_route`; an
/// open one gets it as an `open` event.
pub fn open_window(app: &AppHandle, route: Option<String>) {
    #[cfg(target_os = "macos")]
    dock_icon(app, true);
    if let Some(w) = app.get_webview_window("main") {
        if let Some(r) = route {
            let _ = app.emit("uc-event", json!({"event": "open", "data": {"route": r}}));
        }
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    *tray(app).pending_route.lock().unwrap() = route;
    if let Some(cfg) = app.config().app.windows.iter().find(|w| w.label == "main")
        && let Ok(b) = tauri::WebviewWindowBuilder::from_config(app, cfg)
        && let Ok(w) = b.build()
    {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

// ------------------------------------------------------------------ Linux menu

#[cfg(target_os = "linux")]
struct LinuxMenu {
    menu: tauri::menu::Menu<tauri::Wry>,
    state: tauri::menu::MenuItem<tauri::Wry>,
    pause: tauri::menu::MenuItem<tauri::Wry>,
    stop: tauri::menu::MenuItem<tauri::Wry>,
}

#[cfg(target_os = "linux")]
impl LinuxMenu {
    fn new(app: &AppHandle) -> tauri::Result<LinuxMenu> {
        use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
        let state = MenuItem::with_id(app, "state", "Universal Clipboard", false, None::<&str>)?;
        let open = MenuItem::with_id(app, "open", "Open Universal Clipboard", true, None::<&str>)?;
        let files = MenuItem::with_id(app, "copy-files", "Copy files...", true, None::<&str>)?;
        let folder = MenuItem::with_id(app, "copy-folder", "Copy folder...", true, None::<&str>)?;
        let pause = MenuItem::with_id(app, "pause", "Pause sync", true, None::<&str>)?;
        let stop = MenuItem::with_id(app, "stop", "Stop file copy", false, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Quit (stops syncing)", true, None::<&str>)?;
        let sep = || PredefinedMenuItem::separator(app);
        let menu = Menu::with_items(
            app,
            &[
                &state,
                &sep()?,
                &open,
                &files,
                &folder,
                &sep()?,
                &pause,
                &stop,
                &sep()?,
                &quit,
            ],
        )?;
        Ok(LinuxMenu {
            menu,
            state,
            pause,
            stop,
        })
    }

    fn update(&self, v: &TrayView) {
        let _ = self.state.set_text(match v.pct {
            Some(p) => format!("{} {p} %", v.word),
            None => v.word.clone(),
        });
        let _ = self.pause.set_text(if v.paused {
            "Resume sync"
        } else {
            "Pause sync"
        });
        let _ = self.stop.set_enabled(v.moving);
    }
}

#[cfg(target_os = "linux")]
fn on_menu(app: &AppHandle, id: &str) {
    let app = app.clone();
    match id {
        "open" => {
            tauri::async_runtime::spawn(async move { open_window(&app, None) });
        }
        "copy-files" | "copy-folder" => {
            let folder = id == "copy-folder";
            tauri::async_runtime::spawn(async move {
                let _ = tray_copy(app, folder).await;
            });
        }
        "pause" => {
            let paused = tray(&app)
                .view
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|v| v.paused);
            tauri::async_runtime::spawn(async move {
                let mode = if paused { "off" } else { "forever" };
                let _ = crate::agent_call("pause.set".into(), json!({"mode": mode})).await;
            });
        }
        "stop" => {
            tauri::async_runtime::spawn(async move {
                let _ = crate::agent_call("transfer.stop".into(), json!({})).await;
            });
        }
        "quit" => {
            tauri::async_runtime::spawn(tray_quit(app));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(v: Value) -> TrayView {
        tray_view(Some(&v))
    }

    #[test]
    fn views() {
        assert_eq!(tray_view(None).word, "Not running");
        assert_eq!(tray_view(None).look, Look::Attention);
        let v = st(json!({"state": "working", "paused": false, "transfer": null}));
        assert_eq!(
            (v.look, v.word.as_str(), v.pct),
            (Look::Idle, "Synced", None)
        );
        assert_eq!(v.tooltip(), "Universal Clipboard: Synced");
        let v = st(
            json!({"state": "working", "transfer": {"phase": "receiving", "done": 40, "total": 100}}),
        );
        assert_eq!(
            (v.look, v.word.as_str(), v.pct),
            (Look::Busy, "Receiving", Some(40))
        );
        assert_eq!(v.tooltip(), "Universal Clipboard: Receiving 40 %");
        assert!(v.moving);
        let v = st(
            json!({"state": "working", "transfer": {"phase": "preparing", "done": 0, "total": 0}}),
        );
        assert_eq!(
            (v.look, v.word.as_str(), v.pct),
            (Look::Busy, "Sending", Some(0))
        );
        let v = st(json!({"state": "paused", "paused": true}));
        assert_eq!(
            (v.look, v.word.as_str(), v.paused),
            (Look::Paused, "Paused", true)
        );
        let v = st(json!({"state": "needs-attention", "fix_code": "vpn"}));
        assert_eq!(
            (v.look, v.word.as_str()),
            (Look::Attention, "Not connected")
        );
        let v = st(json!({"state": "needs-attention", "fix_code": "clock"}));
        assert_eq!(v.word, "Needs attention");
        let v = st(json!({"state": "no-peers"}));
        assert_eq!((v.look, v.word.as_str()), (Look::Idle, "Not connected"));
        let v = st(json!({"state": "not-set-up"}));
        assert_eq!(v.word, "Not set up");
        // An attention state wins over a transfer.
        let v = st(json!({"state": "needs-attention", "fix_code": "vpn",
            "transfer": {"phase": "sending", "done": 1, "total": 2}}));
        assert_eq!((v.look, v.pct), (Look::Attention, None));
    }

    const SIZE: (i32, i32) = (368, 440);
    const EDGE: i32 = 14;
    const GAP: i32 = 8;

    fn inside(p: (i32, i32), work: Rect) {
        // The visible panel (without the clear edge) stays in the work area.
        assert!(
            p.0 + EDGE >= work.x && p.0 + SIZE.0 - EDGE <= work.right(),
            "{p:?}"
        );
        assert!(
            p.1 + EDGE >= work.y && p.1 + SIZE.1 - EDGE <= work.bottom(),
            "{p:?}"
        );
    }

    #[test]
    fn panel_above_a_bottom_taskbar() {
        let work = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1032,
        };
        let icon = Rect {
            x: 1700,
            y: 1040,
            w: 24,
            h: 40,
        };
        let p = panel_position(icon, work, SIZE, EDGE, GAP);
        assert_eq!(
            p.1 + SIZE.1 - EDGE,
            1032 - GAP,
            "visible bottom sits GAP above the bar"
        );
        assert_eq!(
            p.0 + SIZE.0 - EDGE,
            1724,
            "right edge on the icon's right edge"
        );
        inside(p, work);
        // An icon at the far right corner: pushed left, still inside.
        let p = panel_position(Rect { x: 1900, ..icon }, work, SIZE, EDGE, GAP);
        inside(p, work);
    }

    #[test]
    fn panel_below_a_top_bar_and_beside_side_bars() {
        // macOS menu bar (or a Windows taskbar on top).
        let work = Rect {
            x: 0,
            y: 25,
            w: 1512,
            h: 957,
        };
        let icon = Rect {
            x: 1300,
            y: 0,
            w: 22,
            h: 24,
        };
        let p = panel_position(icon, work, SIZE, EDGE, GAP);
        assert_eq!(p.1 + EDGE, 25 + GAP);
        inside(p, work);
        // Taskbar on the left and on the right.
        let work = Rect {
            x: 60,
            y: 0,
            w: 1860,
            h: 1080,
        };
        let p = panel_position(
            Rect {
                x: 10,
                y: 900,
                w: 40,
                h: 24,
            },
            work,
            SIZE,
            EDGE,
            GAP,
        );
        assert_eq!(p.0 + EDGE, 60 + GAP);
        inside(p, work);
        let work = Rect {
            x: 0,
            y: 0,
            w: 1860,
            h: 1080,
        };
        let p = panel_position(
            Rect {
                x: 1870,
                y: 900,
                w: 40,
                h: 24,
            },
            work,
            SIZE,
            EDGE,
            GAP,
        );
        assert_eq!(p.0 + SIZE.0 - EDGE, 1860 - GAP);
        inside(p, work);
    }

    #[test]
    fn panel_on_a_second_screen_and_from_the_overflow_flyout() {
        // Second monitor left of the main one, at negative x.
        let work = Rect {
            x: -2560,
            y: 0,
            w: 2560,
            h: 1400,
        };
        let icon = Rect {
            x: -200,
            y: 1410,
            w: 30,
            h: 30,
        };
        let p = panel_position(icon, work, SIZE, EDGE, GAP);
        inside(p, work);
        // Icon in the overflow flyout, inside the work area near the bottom.
        let work = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1032,
        };
        let p = panel_position(
            Rect {
                x: 1600,
                y: 960,
                w: 32,
                h: 32,
            },
            work,
            SIZE,
            EDGE,
            GAP,
        );
        inside(p, work);
        assert!(p.1 + SIZE.1 - EDGE <= 960);
    }

    #[test]
    fn tray_flag() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(tray_arg(&a(&["x", "--tray"])));
        assert!(!tray_arg(&a(&["x", "--route", "update"])));
    }

    #[test]
    fn every_look_has_icons() {
        for l in [Look::Idle, Look::Busy, Look::Paused, Look::Attention] {
            for scale in [1.0, 1.25, 1.5, 2.0] {
                let png = icon_png(l, scale);
                assert!(Image::from_bytes(png).is_ok(), "{l:?} {scale}");
            }
        }
    }
}
