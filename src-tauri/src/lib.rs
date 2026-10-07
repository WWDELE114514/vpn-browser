use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use tauri::{
    webview::WebviewBuilder, window::WindowBuilder, AppHandle, LogicalPosition, LogicalSize,
    Manager, Position, Size, State, WebviewUrl, WindowEvent,
};
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

const TABBAR_H: f64 = 34.0;
const TOOLBAR_H: f64 = 46.0;
const UI_H: f64 = TABBAR_H + TOOLBAR_H;

#[derive(Clone, serde::Serialize)]
struct TabInfo {
    id: u32,
    title: String,
    url: String,
    incognito: bool,
}

struct AppState {
    mixed_port: u16,
    controller_port: u16,
    child: Mutex<Option<CommandChild>>,
    ui_height: Mutex<f64>,
    tabs: Mutex<Vec<TabInfo>>,
    active: Mutex<u32>,
    next_id: AtomicU32,
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(17897)
}

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn default_config(state: &AppState) -> String {
    format!(
        "mixed-port: {}\nexternal-controller: 127.0.0.1:{}\nallow-lan: false\nbind-address: 127.0.0.1\nmode: direct\nlog-level: warning\nproxies: []\nproxy-groups: []\nrules:\n  - MATCH,DIRECT\n",
        state.mixed_port, state.controller_port
    )
}

fn start_core(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let dir = config_dir(app)?;
    let cfg = dir.join("config.yaml");
    if !cfg.exists() {
        fs::write(&cfg, default_config(state)).map_err(|e| e.to_string())?;
    }

    let sidecar = app.shell().sidecar("mihomo").map_err(|e| e.to_string())?;
    let (mut rx, child) = sidecar
        .args(vec![
            "-d".to_string(),
            dir.to_string_lossy().to_string(),
            "-f".to_string(),
            cfg.to_string_lossy().to_string(),
        ])
        .spawn()
        .map_err(|e| e.to_string())?;

    tauri::async_runtime::spawn(async move {
        use tauri_plugin_shell::process::CommandEvent;
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stderr(line) => {
                    let _ = String::from_utf8(line);
                }
                CommandEvent::Terminated(_) => break,
                _ => {}
            }
        }
    });

    *state.child.lock().unwrap() = Some(child);
    Ok(())
}

async fn restart_core(app: &AppHandle, state: &AppState) -> Result<(), String> {
    if let Some(child) = state.child.lock().unwrap().take() {
        let _ = child.kill();
    }
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    start_core(app, state)
}

fn normalize_url(input: &str) -> String {
    let t = input.trim();
    if t.is_empty() {
        return "about:blank".into();
    }
    if t.starts_with("about:") || t.contains("://") {
        return t.to_string();
    }
    if t.contains('.') && !t.contains(' ') {
        format!("https://{t}")
    } else {
        format!("https://www.bing.com/search?q={}", urlencoding::encode(t))
    }
}

fn tabs_json(app: &AppHandle) -> String {
    let st = match app.try_state::<AppState>() {
        Some(s) => s,
        None => return "[]".into(),
    };
    let tabs = st.tabs.lock().unwrap().clone();
    let active = *st.active.lock().unwrap();
    let list: Vec<serde_json::Value> = tabs
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "title": t.title,
                "url": t.url,
                "incognito": t.incognito,
                "active": t.id == active,
            })
        })
        .collect();
    serde_json::to_string(&list).unwrap_or_else(|_| "[]".into())
}

fn notify_tabs(app: &AppHandle) {
    if let Some(ui) = app.get_webview("ui") {
        let json = tabs_json(app);
        let _ = ui.eval(&format!("window.__tabs && window.__tabs({json})"));
    }
}

fn layout(app: &AppHandle) -> Result<(), String> {
    let win = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    let scale = win.scale_factor().unwrap_or(1.0);
    let size = win.inner_size().map_err(|e| e.to_string())?;
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;

    let st = app.state::<AppState>();
    let ui_h = *st.ui_height.lock().unwrap();
    let active = *st.active.lock().unwrap();
    let tabs = st.tabs.lock().unwrap().clone();

    let content_h = (h - ui_h).max(1.0);

    if let Some(ui) = app.get_webview("ui") {
        let _ = ui.set_size(Size::Logical(LogicalSize::new(w, ui_h)));
    }

    for tab in &tabs {
        let label = format!("content-{}", tab.id);
        if let Some(wv) = app.get_webview(&label) {
            if tab.id == active {
                let _ = wv.set_position(Position::Logical(LogicalPosition::new(0.0, ui_h)));
                let _ = wv.set_size(Size::Logical(LogicalSize::new(w, content_h)));
            } else {
                let _ = wv.set_position(Position::Logical(LogicalPosition::new(-30000.0, -30000.0)));
            }
        }
    }
    Ok(())
}

fn active_label(app: &AppHandle) -> Result<String, String> {
    let st = app.state::<AppState>();
    let active = *st.active.lock().unwrap();
    if active == 0 {
        Err("no active tab".into())
    } else {
        Ok(format!("content-{active}"))
    }
}

fn create_tab(app: &AppHandle, url: &str, incognito: bool) -> Result<u32, String> {
    let id = app.state::<AppState>().next_id.fetch_add(1, Ordering::SeqCst);
    let label = format!("content-{id}");
    let win = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    let target = normalize_url(url);
    let parsed = tauri::Url::parse(&target).map_err(|e| e.to_string())?;

    let app_nav = app.clone();
    let app_title = app.clone();

    let builder = WebviewBuilder::new(label.as_str(), WebviewUrl::External(parsed))
        .incognito(incognito)
        .on_navigation(move |u| {
            if let Some(st) = app_nav.try_state::<AppState>() {
                if let Some(t) = st.tabs.lock().unwrap().iter_mut().find(|t| t.id == id) {
                    t.url = u.as_str().to_string();
                }
            }
            notify_tabs(&app_nav);
            true
        })
        .on_document_title_changed(move |_wv, title| {
            if let Some(st) = app_title.try_state::<AppState>() {
                if let Some(t) = st.tabs.lock().unwrap().iter_mut().find(|t| t.id == id) {
                    t.title = title.clone();
                }
            }
            notify_tabs(&app_title);
        });

    win.add_child(
        builder,
        LogicalPosition::new(-30000.0, -30000.0),
        LogicalSize::new(100.0, 100.0),
    )
    .map_err(|e| e.to_string())?;

    {
        let st = app.state::<AppState>();
        st.tabs.lock().unwrap().push(TabInfo {
            id,
            title: if incognito {
                "无痕标签".into()
            } else {
                "新标签".into()
            },
            url: target,
            incognito,
        });
        *st.active.lock().unwrap() = id;
    }

    layout(app)?;
    notify_tabs(app);
    Ok(id)
}

#[tauri::command]
async fn new_tab(app: AppHandle, url: String, incognito: bool) -> Result<u32, String> {
    create_tab(&app, &url, incognito)
}

#[tauri::command]
fn close_tab(app: AppHandle, id: u32) -> Result<(), String> {
    let st = app.state::<AppState>();
    let label = format!("content-{id}");
    if let Some(wv) = app.get_webview(&label) {
        let _ = wv.close();
    }
    {
        let mut tabs = st.tabs.lock().unwrap();
        tabs.retain(|t| t.id != id);
        let mut active = st.active.lock().unwrap();
        if *active == id {
            *active = tabs.last().map(|t| t.id).unwrap_or(0);
        }
    }
    layout(&app)?;
    notify_tabs(&app);
    Ok(())
}

#[tauri::command]
fn activate_tab(app: AppHandle, id: u32) -> Result<(), String> {
    let st = app.state::<AppState>();
    *st.active.lock().unwrap() = id;
    layout(&app)?;
    notify_tabs(&app);
    Ok(())
}

#[tauri::command]
fn list_tabs(app: AppHandle) -> serde_json::Value {
    let st = app.state::<AppState>();
    let tabs = st.tabs.lock().unwrap().clone();
    let active = *st.active.lock().unwrap();
    serde_json::json!({ "tabs": tabs, "active": active })
}

#[tauri::command]
fn navigate(app: AppHandle, url: String) -> Result<(), String> {
    let label = active_label(&app)?;
    let wv = app
        .get_webview(&label)
        .ok_or_else(|| "active webview not found".to_string())?;
    let target = normalize_url(&url);
    let parsed = tauri::Url::parse(&target).map_err(|e| e.to_string())?;
    wv.navigate(parsed).map_err(|e| e.to_string())
}

#[tauri::command]
fn go_back(app: AppHandle) -> Result<(), String> {
    let label = active_label(&app)?;
    let wv = app
        .get_webview(&label)
        .ok_or_else(|| "active webview not found".to_string())?;
    wv.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
fn go_forward(app: AppHandle) -> Result<(), String> {
    let label = active_label(&app)?;
    let wv = app
        .get_webview(&label)
        .ok_or_else(|| "active webview not found".to_string())?;
    wv.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
fn reload(app: AppHandle) -> Result<(), String> {
    let label = active_label(&app)?;
    let wv = app
        .get_webview(&label)
        .ok_or_else(|| "active webview not found".to_string())?;
    wv.eval("location.reload()").map_err(|e| e.to_string())
}

#[tauri::command]
fn get_ports(state: State<'_, AppState>) -> serde_json::Value {
    serde_json::json!({
        "mixed": state.mixed_port,
        "controller": state.controller_port,
    })
}

#[tauri::command]
fn set_ui_height(app: AppHandle, state: State<'_, AppState>, height: f64) -> Result<(), String> {
    let height = height.clamp(UI_H, 2000.0);
    *state.ui_height.lock().unwrap() = height;
    layout(&app)
}

#[tauri::command]
async fn import_config(app: AppHandle, content: String) -> Result<(), String> {
    let mut val: serde_yaml::Value =
        serde_yaml::from_str(&content).map_err(|e| format!("YAML 解析失败: {e}"))?;

    let st = app.state::<AppState>();
    let map = val
        .as_mapping_mut()
        .ok_or_else(|| "配置根节点必须是映射".to_string())?;
    let key = |k: &str| serde_yaml::Value::String(k.to_string());

    map.insert(
        key("mixed-port"),
        serde_yaml::Value::Number((st.mixed_port as u64).into()),
    );
    map.insert(
        key("external-controller"),
        serde_yaml::Value::String(format!("127.0.0.1:{}", st.controller_port)),
    );
    map.insert(key("allow-lan"), serde_yaml::Value::Bool(false));
    map.insert(
        key("bind-address"),
        serde_yaml::Value::String("127.0.0.1".into()),
    );
    map.insert(key("log-level"), serde_yaml::Value::String("warning".into()));
    map.remove(&serde_yaml::Value::String("tun".into()));
    map.remove(&serde_yaml::Value::String("secret".into()));

    let out = serde_yaml::to_string(&val).map_err(|e| e.to_string())?;
    let dir = config_dir(&app)?;
    fs::write(dir.join("config.yaml"), out).map_err(|e| e.to_string())?;

    restart_core(&app, st.inner()).await
}

#[tauri::command]
async fn fetch_subscription(url: String) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .header("User-Agent", "clash-verge/v2.0.0")
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("订阅返回 {}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_mode(app: AppHandle, mode: String) -> Result<(), String> {
    let st = app.state::<AppState>();
    let url = format!("http://127.0.0.1:{}/configs", st.controller_port);
    reqwest::Client::new()
        .patch(url)
        .json(&serde_json::json!({ "mode": mode }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn list_proxies(app: AppHandle) -> Result<serde_json::Value, String> {
    let st = app.state::<AppState>();
    let url = format!("http://127.0.0.1:{}/proxies", st.controller_port);
    let resp = reqwest::get(url).await.map_err(|e| e.to_string())?;
    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn select_proxy(app: AppHandle, group: String, name: String) -> Result<(), String> {
    let st = app.state::<AppState>();
    let url = format!(
        "http://127.0.0.1:{}/proxies/{}",
        st.controller_port,
        urlencoding::encode(&group)
    );
    reqwest::Client::new()
        .put(url)
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn core_status(app: AppHandle) -> serde_json::Value {
    let running = app
        .try_state::<AppState>()
        .map(|st| st.child.lock().unwrap().is_some())
        .unwrap_or(false);
    serde_json::json!({ "running": running })
}

#[tauri::command]
async fn restart_core_cmd(app: AppHandle) -> Result<(), String> {
    let st = app.state::<AppState>();
    restart_core(&app, st.inner()).await
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            new_tab,
            close_tab,
            activate_tab,
            list_tabs,
            navigate,
            go_back,
            go_forward,
            reload,
            get_ports,
            set_ui_height,
            import_config,
            fetch_subscription,
            set_mode,
            list_proxies,
            select_proxy,
            core_status,
            restart_core_cmd,
        ])
        .setup(|app| {
            let mixed_port = free_port();
            let controller_port = free_port();
            app.manage(AppState {
                mixed_port,
                controller_port,
                child: Mutex::new(None),
                ui_height: Mutex::new(UI_H),
                tabs: Mutex::new(Vec::new()),
                active: Mutex::new(0),
                next_id: AtomicU32::new(1),
            });

            // 只让本进程的 WebView2 走内置 mihomo，并关闭非代理 UDP 的 WebRTC 泄漏
            std::env::set_var(
                "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                format!(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
                     --proxy-server=socks5://127.0.0.1:{} \
                     --force-webrtc-ip-handling-policy=disable_non_proxied_udp",
                    mixed_port
                ),
            );

            let window = WindowBuilder::new(app, "main")
                .title("VpnBrowser")
                .inner_size(1200.0, 800.0)
                .min_inner_size(640.0, 420.0)
                .build()?;

            window.add_child(
                WebviewBuilder::new("ui", WebviewUrl::App("index.html".into())),
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(1200.0, UI_H),
            )?;

            let app_for_event = app.handle().clone();
            window.on_window_event(move |event| match event {
                WindowEvent::Resized(_) => {
                    let _ = layout(&app_for_event);
                }
                WindowEvent::Destroyed => {
                    if let Some(st) = app_for_event.try_state::<AppState>() {
                        if let Some(child) = st.child.lock().unwrap().take() {
                            let _ = child.kill();
                        }
                    }
                }
                _ => {}
            });

            let st = app.state::<AppState>();
            if let Err(e) = start_core(app.handle(), st.inner()) {
                eprintln!("failed to start mihomo core: {e}");
            }

            if let Err(e) = create_tab(app.handle(), "https://www.bing.com", false) {
                eprintln!("failed to create initial tab: {e}");
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
