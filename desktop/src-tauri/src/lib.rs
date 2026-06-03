use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use chrono::Local;
use directories::ProjectDirs;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::utils::config::Color;
use tauri::{
    async_runtime::JoinHandle, AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State,
    WindowEvent,
};
use tokio::sync::Mutex;
use url::Url;

#[cfg(target_os = "linux")]
mod native_overlay;

const MAX_LOG_LINES: usize = 200;
const DEFAULT_SERVER_URL: &str = "http://192.168.1.100:8765";
const DEFAULT_FONT_FAMILY: &str = "Sans";
const FALLBACK_FONT_FAMILIES: &[&str] = &["Sans", "Serif", "Monospace"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleEntry {
    pub timestamp: String,
    pub original: String,
    pub translated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayBounds {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: u32,
    pub height: u32,
}

impl Default for OverlayBounds {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: 920,
            height: 260,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub server_url: String,
    pub original_text_color: String,
    pub translated_text_color: String,
    pub background_color: String,
    pub background_opacity: u8,
    pub font_size: f32,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    pub max_subtitle_count: usize,
    pub overlay_bounds: OverlayBounds,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server_url: DEFAULT_SERVER_URL.to_string(),
            original_text_color: "#81D4FA".to_string(),
            translated_text_color: "#FFD54F".to_string(),
            background_color: "#000000".to_string(),
            background_opacity: 70,
            font_size: 16.0,
            font_family: default_font_family(),
            max_subtitle_count: 3,
            overlay_bounds: OverlayBounds::default(),
        }
    }
}

fn default_font_family() -> String {
    DEFAULT_FONT_FAMILY.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeState {
    pub running: bool,
    pub connection_state: String,
    pub subtitles: Vec<SubtitleEntry>,
    pub logs: Vec<String>,
    pub last_error: Option<String>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            running: false,
            connection_state: "disconnected".to_string(),
            subtitles: Vec::new(),
            logs: Vec::new(),
            last_error: None,
        }
    }
}

struct InnerState {
    settings: Settings,
    runtime: RuntimeState,
    stream_task: Option<JoinHandle<()>>,
    config_path: PathBuf,
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Mutex<InnerState>>,
    client: Client,
    #[cfg(target_os = "linux")]
    native_overlay: Arc<StdMutex<Option<native_overlay::NativeOverlayHandle>>>,
}

impl AppState {
    fn load() -> Self {
        let config_path = settings_path();
        let settings = fs::read_to_string(&config_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
            .unwrap_or_default();

        Self {
            inner: Arc::new(Mutex::new(InnerState {
                settings,
                runtime: RuntimeState::default(),
                stream_task: None,
                config_path,
            })),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .user_agent("SubtitleOverlayDesktop/0.1")
                .build()
                .expect("failed to build HTTP client"),
            #[cfg(target_os = "linux")]
            native_overlay: Arc::new(StdMutex::new(None)),
        }
    }
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let inner = state.inner.lock().await;
    Ok(inner.settings.clone())
}

#[tauri::command]
fn list_system_fonts() -> Vec<String> {
    system_font_families()
}

#[tauri::command]
async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    mut settings: Settings,
) -> Result<(), String> {
    normalize_settings(&mut settings);
    let preserve_native_bounds = is_native_overlay_active(&app);

    let (path, subtitles) = {
        let mut inner = state.inner.lock().await;
        if preserve_native_bounds {
            settings.overlay_bounds = inner.settings.overlay_bounds.clone();
        }
        inner.settings = settings.clone();
        let max_count = inner.settings.max_subtitle_count;
        while inner.runtime.subtitles.len() > max_count {
            inner.runtime.subtitles.remove(0);
        }
        (inner.config_path.clone(), inner.runtime.subtitles.clone())
    };

    persist_settings(&path, &settings)?;
    update_native_overlay_settings(&app, settings.clone());
    update_native_overlay_subtitles(&app, subtitles.clone());
    let _ = app.emit("settings-updated", settings);
    let _ = app.emit("subtitle-buffer", subtitles);
    Ok(())
}

#[tauri::command]
async fn start_stream(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let server_url = {
        let mut inner = state.inner.lock().await;
        if let Some(task) = inner.stream_task.take() {
            task.abort();
        }

        inner.runtime.running = true;
        inner.runtime.connection_state = "connecting".to_string();
        inner.runtime.last_error = None;
        inner.settings.server_url.clone()
    };

    let state_for_task = state.inner.clone();
    let client = state.client.clone();
    let app_for_task = app.clone();
    let handle = tauri::async_runtime::spawn(async move {
        stream_loop(app_for_task, state_for_task, client, server_url).await;
    });

    {
        let mut inner = state.inner.lock().await;
        inner.stream_task = Some(handle);
    }

    let _ = app.emit("connection-state", "connecting");
    push_log(&app, &state.inner, "服務啟動請求已發送").await;
    Ok(())
}

#[tauri::command]
async fn stop_stream(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    stop_stream_inner(&app, &state.inner, "已停止字幕串流").await;
    Ok(())
}

#[tauri::command]
async fn get_runtime_state(state: State<'_, AppState>) -> Result<RuntimeState, String> {
    let inner = state.inner.lock().await;
    Ok(inner.runtime.clone())
}

#[tauri::command]
async fn save_overlay_bounds(
    app: AppHandle,
    state: State<'_, AppState>,
    bounds: OverlayBounds,
) -> Result<(), String> {
    let (path, settings) = {
        let mut inner = state.inner.lock().await;
        inner.settings.overlay_bounds = bounds.clone();
        (inner.config_path.clone(), inner.settings.clone())
    };
    persist_settings(&path, &settings)?;
    apply_native_overlay_bounds(&app, bounds);
    Ok(())
}

#[tauri::command]
async fn exit_app(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    stop_stream_inner(&app, &state.inner, "正在退出").await;
    close_native_overlay(&app);
    app.exit(0);
    Ok(())
}

async fn stream_loop(
    app: AppHandle,
    state: Arc<Mutex<InnerState>>,
    client: Client,
    base_url: String,
) {
    let mut retry_delay = Duration::from_secs(1);
    let max_retry_delay = Duration::from_secs(30);

    loop {
        set_connection_state(&app, &state, "connecting").await;

        match get_active_task(&client, &base_url).await {
            Ok(Some(task_id)) => {
                retry_delay = Duration::from_secs(1);
                if let Err(err) =
                    connect_to_stream(&app, &state, &client, &base_url, &task_id).await
                {
                    set_stream_error(&app, &state, format!("連線錯誤: {err}")).await;
                }
            }
            Ok(None) => {
                set_stream_error(&app, &state, "沒有進行中的翻譯任務".to_string()).await;
            }
            Err(err) => {
                set_stream_error(&app, &state, format!("連線錯誤: {err}")).await;
            }
        }

        if !is_running(&state).await {
            break;
        }

        set_connection_state(&app, &state, "reconnecting").await;
        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(max_retry_delay);
    }

    set_connection_state(&app, &state, "disconnected").await;
}

async fn get_active_task(client: &Client, base_url: &str) -> Result<Option<String>, String> {
    let url = endpoint(base_url, "/api/translation/active-task")?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let json = response
        .json::<Value>()
        .await
        .map_err(|err| err.to_string())?;

    if !json
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(None);
    }

    let task_id = json
        .get("task_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();

    if task_id.is_empty() || task_id == "null" {
        Ok(None)
    } else {
        Ok(Some(task_id.to_string()))
    }
}

async fn connect_to_stream(
    app: &AppHandle,
    state: &Arc<Mutex<InnerState>>,
    client: &Client,
    base_url: &str,
    task_id: &str,
) -> Result<(), String> {
    let url = endpoint(base_url, &format!("/api/translation/stream/{task_id}"))?;
    let response = client
        .get(url)
        .header("Accept", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .send()
        .await
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        return Err(format!("SSE 連線失敗: HTTP {}", response.status().as_u16()));
    }

    set_connection_state(app, state, "connected").await;
    push_log(app, state, "SSE 已連線").await;

    let mut stream = response.bytes_stream();
    let mut pending = String::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    while let Some(next) = stream.next().await {
        if !is_running(state).await {
            break;
        }

        let bytes = next.map_err(|err| err.to_string())?;
        pending.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(line_end) = pending.find('\n') {
            let mut line: String = pending.drain(..=line_end).collect();
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }

            process_sse_line(app, state, &mut current_event, &mut current_data, &line).await;
        }
    }

    Ok(())
}

async fn process_sse_line(
    app: &AppHandle,
    state: &Arc<Mutex<InnerState>>,
    current_event: &mut String,
    current_data: &mut String,
    line: &str,
) {
    if line.starts_with(':') {
        return;
    }

    if let Some(event) = line.strip_prefix("event:") {
        *current_event = event.trim().to_string();
        return;
    }

    if let Some(data) = line.strip_prefix("data:") {
        if !current_data.is_empty() {
            current_data.push('\n');
        }
        current_data.push_str(data.trim());
        return;
    }

    if line.is_empty() && !current_data.is_empty() {
        process_event(app, state, current_event, current_data).await;
        current_event.clear();
        current_data.clear();
    }
}

async fn process_event(app: &AppHandle, state: &Arc<Mutex<InnerState>>, event: &str, data: &str) {
    match event {
        "subtitle" => match parse_subtitle(data) {
            Some(entry) => add_subtitle(app, state, entry).await,
            None => set_stream_error(app, state, "解析字幕事件失敗".to_string()).await,
        },
        "status" => {
            if let Ok(json) = serde_json::from_str::<Value>(data) {
                let status = json
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let _ = app.emit("status", json.clone());
                push_log(app, state, &format!("[狀態] status={status}")).await;
            }
        }
        "error" => {
            let message = serde_json::from_str::<Value>(data)
                .ok()
                .and_then(|json| {
                    json.get("message")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| "未知錯誤".to_string());
            set_stream_error(app, state, message).await;
        }
        _ => {
            if let Some(entry) = parse_subtitle(data) {
                add_subtitle(app, state, entry).await;
            }
        }
    }
}

fn parse_subtitle(data: &str) -> Option<SubtitleEntry> {
    let json = serde_json::from_str::<Value>(data).ok()?;
    let original = json
        .get("original")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let translated = json
        .get("translated")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if original.is_empty() && translated.is_empty() {
        return None;
    }

    Some(SubtitleEntry {
        timestamp: json
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        original,
        translated,
    })
}

async fn add_subtitle(app: &AppHandle, state: &Arc<Mutex<InnerState>>, entry: SubtitleEntry) {
    let (subtitles, action) = {
        let mut inner = state.lock().await;
        let updated = try_update_existing(&mut inner.runtime.subtitles, &entry);

        if !updated {
            inner.runtime.subtitles.push(entry.clone());
            let max_count = inner.settings.max_subtitle_count;
            while inner.runtime.subtitles.len() > max_count {
                inner.runtime.subtitles.remove(0);
            }
        }

        (
            inner.runtime.subtitles.clone(),
            if updated { "更新" } else { "新增" },
        )
    };

    update_native_overlay_subtitles(app, subtitles.clone());
    let _ = app.emit("subtitle-buffer", subtitles);
    push_log(
        app,
        state,
        &format!(
            "[字幕/{action}] ts={} | {} | {}",
            entry.timestamp, entry.original, entry.translated
        ),
    )
    .await;
}

fn try_update_existing(entries: &mut [SubtitleEntry], entry: &SubtitleEntry) -> bool {
    if !entry.timestamp.is_empty() {
        if let Some(existing) = entries
            .iter_mut()
            .rev()
            .find(|existing| existing.timestamp == entry.timestamp)
        {
            *existing = entry.clone();
            return true;
        }
    }

    if !entry.original.is_empty() {
        if let Some(last) = entries.last_mut() {
            if last.original == entry.original {
                *last = entry.clone();
                return true;
            }
        }
    }

    false
}

async fn set_connection_state(
    app: &AppHandle,
    state: &Arc<Mutex<InnerState>>,
    connection_state: &str,
) {
    {
        let mut inner = state.lock().await;
        inner.runtime.connection_state = connection_state.to_string();
    }
    let _ = app.emit("connection-state", connection_state);

    let label = match connection_state {
        "connecting" => "連線中...",
        "connected" => "已連線",
        "reconnecting" => "重新連線中...",
        "disconnected" => "已斷線",
        _ => connection_state,
    };
    push_log(app, state, &format!("[連線] {label}")).await;
}

async fn set_stream_error(app: &AppHandle, state: &Arc<Mutex<InnerState>>, message: String) {
    {
        let mut inner = state.lock().await;
        inner.runtime.last_error = Some(message.clone());
    }
    let _ = app.emit("stream-error", message.clone());
    push_log(app, state, &format!("[錯誤] {message}")).await;
}

async fn push_log(app: &AppHandle, state: &Arc<Mutex<InnerState>>, message: &str) {
    let line = format!("[{}] {message}", Local::now().format("%H:%M:%S"));
    {
        let mut inner = state.lock().await;
        inner.runtime.logs.push(line.clone());
        if inner.runtime.logs.len() > MAX_LOG_LINES {
            let overflow = inner.runtime.logs.len() - MAX_LOG_LINES;
            inner.runtime.logs.drain(0..overflow);
        }
    }
    let _ = app.emit("log-line", line);
}

async fn is_running(state: &Arc<Mutex<InnerState>>) -> bool {
    let inner = state.lock().await;
    inner.runtime.running
}

async fn stop_stream_inner(app: &AppHandle, state: &Arc<Mutex<InnerState>>, log_message: &str) {
    let task = {
        let mut inner = state.lock().await;
        inner.runtime.running = false;
        inner.runtime.connection_state = "disconnected".to_string();
        inner.stream_task.take()
    };

    if let Some(task) = task {
        task.abort();
    }

    let _ = app.emit("connection-state", "disconnected");
    push_log(app, state, log_message).await;
}

fn endpoint(base_url: &str, path: &str) -> Result<Url, String> {
    let normalized = normalize_server_url(base_url);
    let base = Url::parse(&normalized).map_err(|err| format!("伺服器位址無效: {err}"))?;
    base.join(path.trim_start_matches('/'))
        .map_err(|err| format!("端點組合失敗: {err}"))
}

fn normalize_settings(settings: &mut Settings) {
    settings.server_url = normalize_server_url(&settings.server_url);
    settings.background_opacity = settings.background_opacity.min(100);
    settings.font_size = settings.font_size.clamp(10.0, 32.0);
    settings.font_family = normalize_font_family(&settings.font_family);
    settings.max_subtitle_count = settings.max_subtitle_count.clamp(1, 10);
    settings.overlay_bounds.width = settings.overlay_bounds.width.max(280);
    settings.overlay_bounds.height = settings.overlay_bounds.height.max(88);
}

fn normalize_font_family(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        default_font_family()
    } else {
        normalized.chars().take(120).collect()
    }
}

fn system_font_families() -> Vec<String> {
    let mut families = FALLBACK_FONT_FAMILIES
        .iter()
        .map(|family| (*family).to_string())
        .collect::<BTreeSet<_>>();

    let mut database = fontdb::Database::new();
    database.load_system_fonts();

    for face in database.faces() {
        for (family, _) in &face.families {
            families.insert(normalize_font_family(family));
        }
    }

    families.into_iter().collect()
}

fn normalize_server_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return DEFAULT_SERVER_URL.to_string();
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

fn settings_path() -> PathBuf {
    if let Some(project_dirs) = ProjectDirs::from("com", "translator", "SubtitleOverlay") {
        return project_dirs.config_dir().join("desktop-settings.json");
    }

    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("desktop-settings.json")
}

fn persist_settings(path: &PathBuf, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let text = serde_json::to_string_pretty(settings).map_err(|err| err.to_string())?;
    fs::write(path, text).map_err(|err| err.to_string())
}

#[cfg(target_os = "linux")]
fn update_native_overlay_settings(app: &AppHandle, settings: Settings) {
    if let Some(overlay) = native_overlay_handle(app) {
        overlay.update_settings(settings);
    }
}

#[cfg(not(target_os = "linux"))]
fn update_native_overlay_settings(_app: &AppHandle, _settings: Settings) {}

#[cfg(target_os = "linux")]
fn update_native_overlay_subtitles(app: &AppHandle, subtitles: Vec<SubtitleEntry>) {
    if let Some(overlay) = native_overlay_handle(app) {
        overlay.update_subtitles(subtitles);
    }
}

#[cfg(not(target_os = "linux"))]
fn update_native_overlay_subtitles(_app: &AppHandle, _subtitles: Vec<SubtitleEntry>) {}

#[cfg(target_os = "linux")]
fn apply_native_overlay_bounds(app: &AppHandle, bounds: OverlayBounds) {
    if let Some(overlay) = native_overlay_handle(app) {
        overlay.apply_bounds(bounds);
    }
}

#[cfg(not(target_os = "linux"))]
fn apply_native_overlay_bounds(_app: &AppHandle, _bounds: OverlayBounds) {}

#[cfg(target_os = "linux")]
fn close_native_overlay(app: &AppHandle) {
    if let Some(overlay) = native_overlay_handle(app) {
        overlay.close();
    }
}

#[cfg(not(target_os = "linux"))]
fn close_native_overlay(_app: &AppHandle) {}

#[cfg(target_os = "linux")]
fn native_overlay_handle(app: &AppHandle) -> Option<native_overlay::NativeOverlayHandle> {
    let state = app.state::<AppState>();
    state
        .native_overlay
        .lock()
        .ok()
        .and_then(|overlay| overlay.clone())
}

#[cfg(target_os = "linux")]
fn is_native_overlay_active(app: &AppHandle) -> bool {
    native_overlay_handle(app).is_some()
}

#[cfg(not(target_os = "linux"))]
fn is_native_overlay_active(_app: &AppHandle) -> bool {
    false
}

pub fn run() {
    tauri::Builder::default()
        .manage(AppState::load())
        .on_window_event(|window, event| {
            if window.label() == "settings" && matches!(event, WindowEvent::CloseRequested { .. }) {
                close_native_overlay(window.app_handle());
                window.app_handle().exit(0);
            }
        })
        .setup(|app| {
            let state = app.state::<AppState>();
            let (settings, subtitles) = tauri::async_runtime::block_on(async {
                let inner = state.inner.lock().await;
                (inner.settings.clone(), inner.runtime.subtitles.clone())
            });
            let use_native_overlay = should_use_native_overlay();

            if let Some(window) = app.get_webview_window("overlay") {
                if use_native_overlay {
                    let _ = window.hide();
                } else {
                    let _ = window.set_background_color(Some(Color(0, 0, 0, 0)));
                    prepare_linux_overlay_transparency(&window);
                    let bounds = settings.overlay_bounds.clone();
                    let _ = window.set_size(PhysicalSize::new(bounds.width, bounds.height));
                    if let (Some(x), Some(y)) = (bounds.x, bounds.y) {
                        let _ = window.set_position(PhysicalPosition::new(x, y));
                    }
                    let _ = window.show();
                }
            }

            setup_native_overlay(app, use_native_overlay, settings, subtitles);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            list_system_fonts,
            save_settings,
            start_stream,
            stop_stream,
            get_runtime_state,
            save_overlay_bounds,
            exit_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running SubtitleOverlay desktop app");
}

#[cfg(target_os = "linux")]
fn should_use_native_overlay() -> bool {
    native_overlay::should_use_native_overlay()
}

#[cfg(not(target_os = "linux"))]
fn should_use_native_overlay() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn setup_native_overlay(
    app: &mut tauri::App,
    use_native_overlay: bool,
    settings: Settings,
    subtitles: Vec<SubtitleEntry>,
) {
    if !use_native_overlay {
        return;
    }

    let state = app.state::<AppState>();
    let state_for_bounds = state.inner.clone();
    let overlay = native_overlay::spawn(settings, subtitles, move |bounds| {
        let state_for_bounds = state_for_bounds.clone();
        tauri::async_runtime::spawn(async move {
            let (path, settings) = {
                let mut inner = state_for_bounds.lock().await;
                inner.settings.overlay_bounds = bounds;
                (inner.config_path.clone(), inner.settings.clone())
            };

            if let Err(err) = persist_settings(&path, &settings) {
                eprintln!("failed to persist native overlay bounds: {err}");
            }
        });
    });

    let native_overlay_slot = state.native_overlay.clone();
    if let Ok(mut slot) = native_overlay_slot.lock() {
        *slot = Some(overlay);
    };
}

#[cfg(not(target_os = "linux"))]
fn setup_native_overlay(
    _app: &mut tauri::App,
    _use_native_overlay: bool,
    _settings: Settings,
    _subtitles: Vec<SubtitleEntry>,
) {
}

#[cfg(target_os = "linux")]
fn prepare_linux_overlay_transparency<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    use gtk::prelude::*;
    use webkit2gtk::WebViewExt;

    let css_provider = gtk::CssProvider::new();
    if let Err(err) = css_provider.load_from_data(
        b"* {
            background: transparent;
            background-color: transparent;
        }",
    ) {
        eprintln!("failed to load transparent overlay GTK CSS: {err}");
    }

    if let Ok(gtk_window) = window.gtk_window() {
        gtk_window.set_app_paintable(true);
        if let Some(screen) = gtk::prelude::WidgetExt::screen(&gtk_window) {
            if let Some(visual) = screen.rgba_visual() {
                gtk_window.set_visual(Some(&visual));
            }
        }
        gtk_window
            .style_context()
            .add_provider(&css_provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }

    if let Ok(vbox) = window.default_vbox() {
        vbox.set_app_paintable(true);
        vbox.style_context()
            .add_provider(&css_provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }

    if let Err(err) = window.as_ref().with_webview(|webview| {
        let transparent = gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0);
        let css_provider = gtk::CssProvider::new();
        if let Err(err) = css_provider.load_from_data(
            b"* {
                background: transparent;
                background-color: transparent;
            }",
        ) {
            eprintln!("failed to load transparent overlay webview GTK CSS: {err}");
        }

        let webview = webview.inner();
        webview.set_app_paintable(true);
        webview
            .style_context()
            .add_provider(&css_provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        webview.set_background_color(&transparent);
    }) {
        eprintln!("failed to prepare transparent overlay webview: {err}");
    }
}

#[cfg(not(target_os = "linux"))]
fn prepare_linux_overlay_transparency<R: tauri::Runtime>(_window: &tauri::WebviewWindow<R>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_server_url() {
        assert_eq!(
            normalize_server_url(" 192.168.1.100:8765/ "),
            "http://192.168.1.100:8765"
        );
        assert_eq!(
            normalize_server_url("https://example.com:8765/"),
            "https://example.com:8765"
        );
        assert_eq!(normalize_server_url(""), DEFAULT_SERVER_URL);
    }

    #[test]
    fn clamps_settings_to_supported_ranges() {
        let mut settings = Settings {
            background_opacity: 180,
            font_size: 4.0,
            font_family: "   ".to_string(),
            max_subtitle_count: 99,
            overlay_bounds: OverlayBounds {
                x: Some(1),
                y: Some(2),
                width: 100,
                height: 20,
            },
            ..Settings::default()
        };

        normalize_settings(&mut settings);

        assert_eq!(settings.background_opacity, 100);
        assert_eq!(settings.font_size, 10.0);
        assert_eq!(settings.font_family, DEFAULT_FONT_FAMILY);
        assert_eq!(settings.max_subtitle_count, 10);
        assert_eq!(settings.overlay_bounds.width, 280);
        assert_eq!(settings.overlay_bounds.height, 88);
    }

    #[test]
    fn deserializes_legacy_settings_without_font_family() {
        let settings = serde_json::from_str::<Settings>(
            r##"{
                "serverUrl": "http://localhost:8765",
                "originalTextColor": "#ffffff",
                "translatedTextColor": "#ffff00",
                "backgroundColor": "#000000",
                "backgroundOpacity": 70,
                "fontSize": 16,
                "maxSubtitleCount": 3,
                "overlayBounds": { "x": null, "y": null, "width": 920, "height": 260 }
            }"##,
        )
        .expect("legacy settings should deserialize");

        assert_eq!(settings.font_family, DEFAULT_FONT_FAMILY);
    }

    #[test]
    fn updates_subtitle_by_timestamp() {
        let mut entries = vec![SubtitleEntry {
            timestamp: "01".to_string(),
            original: "hello".to_string(),
            translated: "old".to_string(),
        }];

        let updated = SubtitleEntry {
            timestamp: "01".to_string(),
            original: "hello".to_string(),
            translated: "new".to_string(),
        };

        assert!(try_update_existing(&mut entries, &updated));
        assert_eq!(entries[0].translated, "new");
    }

    #[test]
    fn updates_last_subtitle_by_original() {
        let mut entries = vec![SubtitleEntry {
            timestamp: "".to_string(),
            original: "same".to_string(),
            translated: "old".to_string(),
        }];

        let updated = SubtitleEntry {
            timestamp: "".to_string(),
            original: "same".to_string(),
            translated: "new".to_string(),
        };

        assert!(try_update_existing(&mut entries, &updated));
        assert_eq!(entries[0].translated, "new");
    }

    #[test]
    fn parses_subtitle_event_payload() {
        let entry =
            parse_subtitle(r#"{"timestamp":"12:00","original":"こんにちは","translated":"你好"}"#)
                .expect("subtitle payload should parse");

        assert_eq!(entry.timestamp, "12:00");
        assert_eq!(entry.original, "こんにちは");
        assert_eq!(entry.translated, "你好");
    }
}
