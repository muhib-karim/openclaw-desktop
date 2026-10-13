use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::net::TcpStream;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, Runtime,
};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const GATEWAY_PORT: u16 = 18789;

#[derive(Debug, Serialize, Deserialize)]
pub struct GatewayDiagnostics {
    pub openclaw_installed: bool,
    pub gateway_running: bool,
    pub gateway_port: u16,
    pub dashboard_url: String,
    pub openclaw_version: Option<String>,
    pub profile_name: Option<String>,
    pub log_path: String,
    pub error_log_path: String,
}

fn openclaw_home_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not find home directory")?;
    let openclaw_dir = home.join(".openclaw");
    if !openclaw_dir.exists() {
        fs::create_dir_all(&openclaw_dir)
            .map_err(|e| format!("Failed to create OpenClaw directory: {e}"))?;
    }
    Ok(openclaw_dir)
}

fn gateway_log_paths() -> Result<(PathBuf, PathBuf), String> {
    let openclaw_dir = openclaw_home_dir()?;
    Ok((
        openclaw_dir.join("gateway.log"),
        openclaw_dir.join("gateway_error.log"),
    ))
}

fn openclaw_command() -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "openclaw"]);
        cmd
    }
    #[cfg(not(target_os = "windows"))]
    {
        Command::new("openclaw")
    }
}

fn run_openclaw_output(args: &[&str]) -> Result<std::process::Output, String> {
    openclaw_command()
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run openclaw {}: {}", args.join(" "), e))
}

fn run_openclaw_gateway_control(action: &str) -> Result<String, String> {
    let output = run_openclaw_output(&["daemon", action])?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            Ok(format!("Gateway {action} command sent"))
        } else {
            Ok(stdout)
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("openclaw gateway {action} failed"))
        } else {
            Err(stderr)
        }
    }
}

fn start_gateway_foreground_to_logs() -> Result<(), String> {
    let (log_path, error_log_path) = gateway_log_paths()?;

    let stdout_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .map_err(|e| format!("Failed to open gateway log file: {e}"))?;

    let stderr_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&error_log_path)
        .map_err(|e| format!("Failed to open gateway error log file: {e}"))?;

    let mut command = openclaw_command();
    command
        .args(["gateway", "--port", &GATEWAY_PORT.to_string(), "--verbose"])
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));

    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
        .spawn()
        .map_err(|e| format!("Failed to start gateway: {e}"))?;

    Ok(())
}

fn detect_openclaw_version() -> Option<String> {
    let output = run_openclaw_output(&["--version"]).ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Read the gateway token from OpenClaw config file
fn read_gateway_token() -> Option<String> {
    // Try to find the config file
    let home = dirs::home_dir()?;
    let config_path = home.join(".openclaw").join("openclaw.json");

    if !config_path.exists() {
        // Try legacy path
        let legacy_path = home.join(".clawdbot").join("clawdbot.json");
        if legacy_path.exists() {
            return read_token_from_file(&legacy_path);
        }
        return None;
    }

    read_token_from_file(&config_path)
}

fn read_token_from_file(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    parse_gateway_token(&content)
}

/// Extract `gateway.auth.token` from the contents of an OpenClaw config file
fn parse_gateway_token(content: &str) -> Option<String> {
    let json: Value = serde_json::from_str(content).ok()?;
    json.get("gateway")
        .and_then(|g| g.get("auth"))
        .and_then(|a| a.get("token"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GatewayStatus {
    pub running: bool,
    pub port: u16,
    pub dashboard_url: String,
}

fn dashboard_base_url() -> String {
    format!("http://127.0.0.1:{GATEWAY_PORT}/")
}

/// Append the gateway auth token (URL-encoded) to the dashboard URL when available
fn build_dashboard_url(base_url: &str, token: Option<&str>) -> String {
    match token {
        Some(token) => format!("{}?token={}", base_url, urlencoding::encode(token)),
        None => base_url.to_string(),
    }
}

/// Return the last `max_lines` lines of `content`
fn tail_lines(content: &str, max_lines: usize) -> String {
    let log_lines: Vec<&str> = content.lines().collect();
    let start = log_lines.len().saturating_sub(max_lines);
    log_lines[start..].join("\n")
}

/// Check if the OpenClaw gateway is running by probing the port
fn is_gateway_running() -> bool {
    TcpStream::connect(format!("127.0.0.1:{GATEWAY_PORT}")).is_ok()
}

/// Get the current gateway status
#[tauri::command]
fn get_gateway_status() -> GatewayStatus {
    GatewayStatus {
        running: is_gateway_running(),
        port: GATEWAY_PORT,
        dashboard_url: dashboard_base_url(),
    }
}

/// Start the OpenClaw gateway
#[tauri::command]
fn start_gateway() -> Result<String, String> {
    if is_gateway_running() {
        return Ok("Gateway is already running".to_string());
    }

    start_gateway_foreground_to_logs()?;

    Ok("Gateway starting...".to_string())
}

/// Stop the OpenClaw gateway
#[tauri::command]
fn stop_gateway() -> Result<String, String> {
    run_openclaw_gateway_control("stop")
}

/// Restart the OpenClaw gateway
#[tauri::command]
fn restart_gateway() -> Result<String, String> {
    run_openclaw_gateway_control("restart")
}

/// Auto-start gateway if not already running (called on app launch)
#[tauri::command]
fn auto_start_gateway() -> Result<bool, String> {
    if is_gateway_running() {
        Ok(false) // already running
    } else {
        start_gateway_foreground_to_logs()?;
        Ok(true) // started
    }
}

/// Check if OpenClaw is installed
#[tauri::command]
fn is_openclaw_installed() -> bool {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "where", "openclaw"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[cfg(not(target_os = "windows"))]
    {
        Command::new("which")
            .arg("openclaw")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[tauri::command]
fn get_gateway_diagnostics() -> Result<GatewayDiagnostics, String> {
    let (log_path, error_log_path) = gateway_log_paths()?;
    Ok(GatewayDiagnostics {
        openclaw_installed: is_openclaw_installed(),
        gateway_running: is_gateway_running(),
        gateway_port: GATEWAY_PORT,
        dashboard_url: dashboard_base_url(),
        openclaw_version: detect_openclaw_version(),
        profile_name: std::env::var("OPENCLAW_PROFILE").ok(),
        log_path: log_path.display().to_string(),
        error_log_path: error_log_path.display().to_string(),
    })
}

#[tauri::command]
fn run_openclaw_doctor() -> Result<String, String> {
    let output = run_openclaw_output(&["doctor"])?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        if stdout.trim().is_empty() {
            Ok("OpenClaw doctor finished with no output".to_string())
        } else {
            Ok(stdout)
        }
    } else {
        let mut msg = String::from("OpenClaw doctor failed");
        if !stderr.trim().is_empty() {
            msg.push_str("\n\n");
            msg.push_str(stderr.trim());
        }
        if !stdout.trim().is_empty() {
            msg.push_str("\n\n");
            msg.push_str(stdout.trim());
        }
        Err(msg)
    }
}

/// Install OpenClaw via npm
#[tauri::command]
async fn install_openclaw() -> Result<String, String> {
    let output = Command::new("npm")
        .args(["install", "-g", "openclaw"])
        .output()
        .map_err(|e| format!("Failed to install: {e}"))?;

    if output.status.success() {
        Ok("OpenClaw installed successfully".to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// Get the dashboard URL with auth token for iframe embedding
#[tauri::command]
fn get_dashboard_url() -> String {
    build_dashboard_url(&dashboard_base_url(), read_gateway_token().as_deref())
}

/// Navigate main window to the dashboard
#[tauri::command]
async fn open_dashboard_window(app: tauri::AppHandle) -> Result<(), String> {
    // Get the main window and navigate it to the dashboard
    if let Some(window) = app.get_webview_window("main") {
        // Build tokenized URL for authentication
        let dashboard_url =
            build_dashboard_url(&dashboard_base_url(), read_gateway_token().as_deref());
        let url = dashboard_url
            .parse::<tauri::Url>()
            .map_err(|e| format!("Invalid dashboard URL: {e}"))?;
        window
            .navigate(url)
            .map_err(|e| format!("Failed to navigate: {e}"))?;
    }
    Ok(())
}

/// Get the gateway logs from the log file
#[tauri::command]
fn get_gateway_logs(lines: Option<usize>) -> Result<String, String> {
    let home = dirs::home_dir().ok_or("Could not find home directory")?;
    let log_path = home.join(".openclaw").join("gateway.log");

    if !log_path.exists() {
        return Ok("No logs available yet. Start the gateway to see logs.".to_string());
    }

    let content =
        fs::read_to_string(&log_path).map_err(|e| format!("Failed to read log file: {e}"))?;

    Ok(tail_lines(&content, lines.unwrap_or(100)))
}

/// Clear the gateway logs
#[tauri::command]
fn clear_gateway_logs() -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Could not find home directory")?;
    let log_path = home.join(".openclaw").join("gateway.log");

    if log_path.exists() {
        fs::write(&log_path, "").map_err(|e| format!("Failed to clear logs: {e}"))?;
    }

    Ok(())
}

fn create_tray_menu<R: Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
    let status = if is_gateway_running() {
        "🟢 Running"
    } else {
        "🔴 Stopped"
    };

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "status", status, false, None::<&str>)?,
            &MenuItem::with_id(app, "separator", "─────────", false, None::<&str>)?,
            &MenuItem::with_id(app, "start", "▶ Start Gateway", true, None::<&str>)?,
            &MenuItem::with_id(app, "stop", "⏹ Stop Gateway", true, None::<&str>)?,
            &MenuItem::with_id(app, "dashboard", "🌐 Open Dashboard", true, None::<&str>)?,
            &MenuItem::with_id(app, "separator2", "─────────", false, None::<&str>)?,
            &MenuItem::with_id(app, "quit", "✖ Quit", true, None::<&str>)?,
        ],
    )?;

    Ok(menu)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Focus existing window instead of opening a duplicate
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            // Create system tray
            let menu = create_tray_menu(app.handle())?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "start" => {
                        let _ = start_gateway();
                    }
                    "stop" => {
                        let _ = stop_gateway();
                    }
                    "dashboard" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
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

            // Handle window close - minimize to tray instead of quitting
            let main_window = app.get_webview_window("main").unwrap();
            let main_window_clone = main_window.clone();
            main_window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    // Prevent the window from closing
                    api.prevent_close();
                    // Hide the window instead
                    let _ = main_window_clone.hide();
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_gateway_status,
            start_gateway,
            stop_gateway,
            restart_gateway,
            auto_start_gateway,
            get_dashboard_url,
            is_openclaw_installed,
            install_openclaw,
            open_dashboard_window,
            get_gateway_logs,
            clear_gateway_logs,
            get_gateway_diagnostics,
            run_openclaw_doctor,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gateway_token_reads_nested_token() {
        let config = r#"{"gateway": {"auth": {"token": "abc123"}}}"#;
        assert_eq!(parse_gateway_token(config), Some("abc123".to_string()));
    }

    #[test]
    fn parse_gateway_token_handles_missing_or_invalid_data() {
        assert_eq!(parse_gateway_token(r#"{"gateway": {}}"#), None);
        assert_eq!(
            parse_gateway_token(r#"{"gateway": {"auth": {"token": 42}}}"#),
            None
        );
        assert_eq!(parse_gateway_token("not json"), None);
    }

    #[test]
    fn read_token_from_file_returns_none_for_missing_file() {
        let path = Path::new("/nonexistent/openclaw-desktop/openclaw.json");
        assert_eq!(read_token_from_file(&path), None);
    }

    #[test]
    fn dashboard_base_url_uses_gateway_port() {
        assert_eq!(dashboard_base_url(), "http://127.0.0.1:18789/");
    }

    #[test]
    fn build_dashboard_url_without_token_is_base_url() {
        assert_eq!(
            build_dashboard_url("http://127.0.0.1:18789/", None),
            "http://127.0.0.1:18789/"
        );
    }

    #[test]
    fn build_dashboard_url_encodes_token() {
        assert_eq!(
            build_dashboard_url("http://127.0.0.1:18789/", Some("a b&c")),
            "http://127.0.0.1:18789/?token=a%20b%26c"
        );
    }

    #[test]
    fn dashboard_url_parses_as_tauri_url() {
        let url = build_dashboard_url(&dashboard_base_url(), Some("t/k"));
        assert!(url.parse::<tauri::Url>().is_ok());
    }

    #[test]
    fn tail_lines_keeps_last_lines() {
        assert_eq!(tail_lines("a\nb\nc\nd", 2), "c\nd");
    }

    #[test]
    fn tail_lines_returns_everything_when_short() {
        assert_eq!(tail_lines("a\nb", 10), "a\nb");
        assert_eq!(tail_lines("", 5), "");
    }

    #[test]
    fn tail_lines_with_zero_limit_is_empty() {
        assert_eq!(tail_lines("a\nb", 0), "");
    }
}
