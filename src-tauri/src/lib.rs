use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{
    webview::WebviewBuilder, window::WindowBuilder, AppHandle, LogicalPosition, LogicalSize,
    Manager, Position, Size, State, WebviewUrl, WindowEvent,
};
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

const DEFAULT_TOOLBAR_H: f64 = 46.0;

struct AppState {
    mixed_port: u16,
    controller_port: u16,
    child: Mutex<Option<CommandChild>>,
    toolbar_h: Mutex<f64>,
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

#[tauri::command]
fn navigate(app: AppHandle, url: String) -> Result<(), String> {
    let wv = app
        .get_webview("content")
        .ok_or_else(|| "content webview not found".to_string())?;
    let target = normalize_url(&url);
    let parsed = tauri::Url::parse(&target).map_err(|e| e.to_string())?;
    wv.navigate(parsed).map_err(|e| e.to_string())
}

#[tauri::command]
fn go_back(app: AppHandle) -> Result<(), String> {
    let wv = app
        .get_webview("content")
        .ok_or_else(|| "content webview not found".to_string())?;
    wv.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
fn go_forward(app: AppHandle) -> Result<(), String> {
    let wv = app
        .get_webview("content")
        .ok_or_else(|| "content webview not found".to_string())?;
    wv.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
fn reload(app: AppHandle) -> Result<(), String> {
    let wv = app
        .get_webview("content")
        .ok_or_else(|| "content webview not found".to_string())?;
    wv.eval("location.reload()").map_err(|e| e.to_string())
}

#[tauri::command]
fn open_devtools(app: AppHandle) {
    if let Some(wv) = app.get_webview("content") {
        wv.open_devtools();
    }
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
    let height = height.clamp(DEFAULT_TOOLBAR_H, 2000.0);
    *state.toolbar_h.lock().unwrap() = height;

    let win = app
        .get_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    let scale = win.scale_factor().unwrap_or(1.0);
    let size = win.inner_size().map_err(|e| e.to_string())?;
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;

    if let Some(ui) = app.get_webview("ui") {
        ui.set_size(Size::Logical(LogicalSize::new(w, height)))
            .map_err(|e| e.to_string())?;
    }
    if let Some(content) = app.get_webview("content") {
        content
            .set_position(Position::Logical(LogicalPosition::new(0.0, height)))
            .map_err(|e| e.to_string())?;
        content
            .set_size(Size::Logical(LogicalSize::new(w, (h - height).max(1.0))))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
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
    map.insert(
        key("log-level"),
        serde_yaml::Value::String("warning".into()),
    );
    map.remove(serde_yaml::Value::String("tun".into()));
    map.remove(serde_yaml::Value::String("secret".into()));

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
            navigate,
            go_back,
            go_forward,
            reload,
            open_devtools,
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
                toolbar_h: Mutex::new(DEFAULT_TOOLBAR_H),
            });

            // 让 WebView2 只通过内置 mihomo 的 socks5 端口上网（仅影响本进程，不碰系统）
            std::env::set_var(
                "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                format!(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --proxy-server=socks5://127.0.0.1:{}",
                    mixed_port
                ),
            );

            let window = WindowBuilder::new(app, "main")
                .title("VpnBrowser")
                .inner_size(1200.0, 800.0)
                .min_inner_size(640.0, 420.0)
                .build()?;

            WebviewBuilder::new("ui", WebviewUrl::App("index.html".into()))
                .position(0.0, 0.0)
                .size(1200.0, DEFAULT_TOOLBAR_H)
                .build_as_child(&window)?;

            let nav_handle = app.handle().clone();
            WebviewBuilder::new(
                "content",
                WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .position(0.0, DEFAULT_TOOLBAR_H)
            .size(1200.0, 800.0 - DEFAULT_TOOLBAR_H)
            .on_navigation(move |url| {
                if let Some(ui) = nav_handle.get_webview("ui") {
                    let u = url.as_str().replace('\\', "\\\\").replace('\'', "\\'");
                    let _ = ui.eval(&format!("window.__onNav && window.__onNav('{u}')"));
                }
                true
            })
            .build_as_child(&window)?;

            let win_for_event = window.clone();
            let app_for_event = app.handle().clone();
            window.on_window_event(move |event| match event {
                WindowEvent::Resized(size) => {
                    let scale = win_for_event.scale_factor().unwrap_or(1.0);
                    let w = size.width as f64 / scale;
                    let h = size.height as f64 / scale;
                    let toolbar_h = app_for_event
                        .try_state::<AppState>()
                        .map(|st| *st.toolbar_h.lock().unwrap())
                        .unwrap_or(DEFAULT_TOOLBAR_H);
                    if let Some(ui) = win_for_event.get_webview("ui") {
                        let _ = ui.set_size(Size::Logical(LogicalSize::new(w, toolbar_h)));
                    }
                    if let Some(content) = win_for_event.get_webview("content") {
                        let _ = content.set_position(Position::Logical(LogicalPosition::new(
                            0.0, toolbar_h,
                        )));
                        let _ = content.set_size(Size::Logical(LogicalSize::new(
                            w,
                            (h - toolbar_h).max(1.0),
                        )));
                    }
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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
