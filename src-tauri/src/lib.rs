use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::net::{SocketAddr, TcpStream};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, Runtime,
};
use tauri_plugin_opener::OpenerExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const GATEWAY_PORT: u16 = 18789;

/// A closed loopback port can take seconds to refuse on Windows, so never wait on the OS default
const PORT_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// How much of a log file is read to show its last lines
const LOG_TAIL_MAX_BYTES: u64 = 256 * 1024;

/// How often the tray status line is re-checked
const TRAY_STATUS_INTERVAL: Duration = Duration::from_secs(3);

/// The gateway process this app spawned. Kept so a second start cannot race the first one, and so
/// the process is reaped once it exits.
static GATEWAY_CHILD: Mutex<Option<Child>> = Mutex::new(None);

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
        // It will hold the gateway token and logs: keep it private when this app creates it
        restrict_to_owner(&openclaw_dir, 0o700);
    }
    Ok(openclaw_dir)
}

/// Best-effort owner-only permissions
#[cfg(unix)]
fn restrict_to_owner(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(mode)) {
        eprintln!("Could not restrict permissions on {}: {e}", path.display());
    }
}

/// The user profile ACL already keeps these paths private on Windows
#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path, _mode: u32) {}

fn gateway_log_paths() -> Result<(PathBuf, PathBuf), String> {
    let openclaw_dir = openclaw_home_dir()?;
    Ok((
        openclaw_dir.join("gateway.log"),
        openclaw_dir.join("gateway_error.log"),
    ))
}

/// Build a command that never opens a console window on Windows
fn hidden_command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// CLIs installed by npm are `.cmd` shims on Windows. `Command` only resolves `.exe` files by
/// bare name, so those have to go through `cmd /c`.
fn shim_command(name: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut command = hidden_command("cmd");
        command.args(["/c", name]);
        command
    }
    #[cfg(not(target_os = "windows"))]
    {
        hidden_command(name)
    }
}

fn openclaw_command() -> Command {
    shim_command("openclaw")
}

fn npm_command() -> Command {
    shim_command("npm")
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
            Err(format!("openclaw daemon {action} failed"))
        } else {
            Err(stderr)
        }
    }
}

/// Start a fresh log file and return a handle in append mode. An append handle always writes at
/// the real end of the file, so clearing the log while the gateway runs cannot leave a hole in it.
fn open_fresh_log(path: &Path) -> std::io::Result<fs::File> {
    fs::write(path, "")?;
    // Verbose gateway output can include request URLs
    restrict_to_owner(path, 0o600);
    OpenOptions::new().append(true).open(path)
}

fn lock_gateway_child() -> MutexGuard<'static, Option<Child>> {
    // A poisoned lock only means another thread panicked while holding it; the slot is still usable
    GATEWAY_CHILD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// True while the tracked process is alive. Once it has exited it is reaped and the slot cleared.
fn child_still_running(slot: &mut Option<Child>) -> bool {
    let alive = slot
        .as_mut()
        .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
    if !alive {
        *slot = None;
    }
    alive
}

fn start_gateway_foreground_to_logs() -> Result<(), String> {
    let mut slot = lock_gateway_child();
    if child_still_running(&mut slot) {
        // A start is already in flight: a second process would fight it for the port and the logs
        return Ok(());
    }

    let (log_path, error_log_path) = gateway_log_paths()?;

    let stdout_file =
        open_fresh_log(&log_path).map_err(|e| format!("Failed to open gateway log file: {e}"))?;
    let stderr_file = open_fresh_log(&error_log_path)
        .map_err(|e| format!("Failed to open gateway error log file: {e}"))?;

    let child = openclaw_command()
        .args(["gateway", "--port", &GATEWAY_PORT.to_string(), "--verbose"])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .map_err(|e| format!("Failed to start gateway: {e}"))?;

    *slot = Some(child);
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

/// Read the last `max_lines` lines of a log without loading the whole file: only the final
/// `max_bytes` are read, and bytes that are not valid UTF-8 are replaced instead of failing.
fn read_log_tail(path: &Path, max_lines: usize, max_bytes: u64) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))?;

    let mut bytes = Vec::new();
    file.take(max_bytes).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);

    // A read that starts mid-file almost always starts mid-line: drop that partial first line
    let visible: &str = if start > 0 {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &text
    };
    Ok(tail_lines(visible, max_lines))
}

/// Gateway output, followed by the error log when that has any content. Startup failures are
/// written to stderr, so without this the log panel would show nothing useful for them.
fn compose_gateway_logs(output: &str, errors: &str) -> String {
    if errors.trim().is_empty() {
        return output.to_string();
    }
    format!("{output}\n\n===== gateway_error.log =====\n{errors}")
        .trim()
        .to_string()
}

/// Check whether something accepts connections on a loopback port, waiting at most `timeout`
fn is_port_open(port: u16, timeout: Duration) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, timeout).is_ok()
}

/// Check if the OpenClaw gateway is running by probing the port
fn is_gateway_running() -> bool {
    is_port_open(GATEWAY_PORT, PORT_PROBE_TIMEOUT)
}

/// Sort nvm version directory names (`v22.3.0`) newest first
#[cfg(not(target_os = "windows"))]
fn sort_node_versions_newest_first(mut names: Vec<String>) -> Vec<String> {
    fn version_key(name: &str) -> Vec<u32> {
        name.trim_start_matches('v')
            .split('.')
            .filter_map(|part| part.parse().ok())
            .collect()
    }
    names.sort_by_key(|name| std::cmp::Reverse(version_key(name)));
    names
}

/// Directories where Node, npm and globally installed CLIs usually live
#[cfg(not(target_os = "windows"))]
fn extra_path_candidates(home: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
        home.join(".npm-global/bin"),
        home.join(".local/bin"),
        home.join(".volta/bin"),
        home.join(".bun/bin"),
        home.join("Library/pnpm"),
        home.join(".local/share/pnpm"),
        home.join(".local/share/fnm/aliases/default/bin"),
        home.join("Library/Application Support/fnm/aliases/default/bin"),
    ];

    // nvm keeps one directory per installed Node version
    let nvm_versions = home.join(".nvm/versions/node");
    if let Ok(entries) = fs::read_dir(&nvm_versions) {
        let names = entries
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        for name in sort_node_versions_newest_first(names) {
            candidates.push(nvm_versions.join(name).join("bin"));
        }
    }

    candidates
}

/// Append each candidate that exists and is not already listed, keeping the current order first
#[cfg(not(target_os = "windows"))]
fn merge_path_dirs(
    current: Vec<PathBuf>,
    candidates: Vec<PathBuf>,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let mut merged = current;
    for dir in candidates {
        if exists(&dir) && !merged.contains(&dir) {
            merged.push(dir);
        }
    }
    merged
}

/// Apps launched from Finder or a desktop launcher do not inherit the login shell `PATH`, so
/// `openclaw`, `npm` and `node` installed through Homebrew, nvm, volta and friends are not found.
/// Append the usual locations once at startup; every command this app spawns inherits them.
#[cfg(not(target_os = "windows"))]
fn augment_process_path() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let current: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    let merged = merge_path_dirs(current, extra_path_candidates(&home), Path::is_dir);
    match std::env::join_paths(merged) {
        Ok(path) => std::env::set_var("PATH", path),
        Err(e) => eprintln!("Could not extend PATH: {e}"),
    }
}

/// Run blocking work (process spawns, socket and file IO) on the blocking thread pool. Commands
/// that are not `async` run on the main thread, where this work would freeze the window and tray.
async fn run_blocking<T, F>(task: F) -> Result<T, String>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|e| format!("Background task failed: {e}"))
}

fn gateway_status() -> GatewayStatus {
    GatewayStatus {
        running: is_gateway_running(),
        port: GATEWAY_PORT,
        dashboard_url: dashboard_base_url(),
    }
}

/// Get the current gateway status
#[tauri::command]
async fn get_gateway_status() -> Result<GatewayStatus, String> {
    run_blocking(gateway_status).await
}

fn start_gateway_blocking() -> Result<String, String> {
    if is_gateway_running() {
        return Ok("Gateway is already running".to_string());
    }

    start_gateway_foreground_to_logs()?;

    Ok("Gateway starting...".to_string())
}

/// Start the OpenClaw gateway
#[tauri::command]
async fn start_gateway() -> Result<String, String> {
    run_blocking(start_gateway_blocking).await?
}

/// Stop the OpenClaw gateway
#[tauri::command]
async fn stop_gateway() -> Result<String, String> {
    run_blocking(|| run_openclaw_gateway_control("stop")).await?
}

/// Restart the OpenClaw gateway
#[tauri::command]
async fn restart_gateway() -> Result<String, String> {
    run_blocking(|| run_openclaw_gateway_control("restart")).await?
}

fn auto_start_gateway_blocking() -> Result<bool, String> {
    if is_gateway_running() {
        Ok(false) // already running
    } else {
        start_gateway_foreground_to_logs()?;
        Ok(true) // started
    }
}

/// Auto-start gateway if not already running (called on app launch)
#[tauri::command]
async fn auto_start_gateway() -> Result<bool, String> {
    run_blocking(auto_start_gateway_blocking).await?
}

fn openclaw_installed() -> bool {
    #[cfg(target_os = "windows")]
    let output = hidden_command("cmd")
        .args(["/c", "where", "openclaw"])
        .output();

    #[cfg(not(target_os = "windows"))]
    let output = hidden_command("which").arg("openclaw").output();

    output.map(|o| o.status.success()).unwrap_or(false)
}

/// Check if OpenClaw is installed
#[tauri::command]
async fn is_openclaw_installed() -> Result<bool, String> {
    run_blocking(openclaw_installed).await
}

fn gateway_diagnostics() -> Result<GatewayDiagnostics, String> {
    let (log_path, error_log_path) = gateway_log_paths()?;
    Ok(GatewayDiagnostics {
        openclaw_installed: openclaw_installed(),
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
async fn get_gateway_diagnostics() -> Result<GatewayDiagnostics, String> {
    run_blocking(gateway_diagnostics).await?
}

fn run_doctor_blocking() -> Result<String, String> {
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

#[tauri::command]
async fn run_openclaw_doctor() -> Result<String, String> {
    run_blocking(run_doctor_blocking).await?
}

fn install_openclaw_blocking() -> Result<String, String> {
    let output = npm_command()
        .args(["install", "-g", "openclaw"])
        .output()
        .map_err(|e| format!("Failed to run npm (is Node.js installed?): {e}"))?;

    if output.status.success() {
        Ok("OpenClaw installed successfully".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err("npm install -g openclaw failed".to_string())
        } else {
            Err(stderr)
        }
    }
}

/// Install OpenClaw via npm
#[tauri::command]
async fn install_openclaw() -> Result<String, String> {
    run_blocking(install_openclaw_blocking).await?
}

/// Dashboard URL carrying the gateway auth token, when one is configured
fn tokenized_dashboard_url() -> String {
    build_dashboard_url(&dashboard_base_url(), read_gateway_token().as_deref())
}

/// Get the dashboard URL with auth token
#[tauri::command]
fn get_dashboard_url() -> String {
    tokenized_dashboard_url()
}

/// Open the dashboard in the default browser. The tokenized URL is built and opened here, so the
/// token never has to be handed to the webview.
#[tauri::command]
async fn open_dashboard_in_browser(app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(tokenized_dashboard_url(), None::<&str>)
        .map_err(|e| format!("Failed to open the dashboard in a browser: {e}"))
}

/// Navigate main window to the dashboard
#[tauri::command]
async fn open_dashboard_window(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("Main window is not available")?;
    let url = tokenized_dashboard_url()
        .parse::<tauri::Url>()
        .map_err(|e| format!("Invalid dashboard URL: {e}"))?;
    window
        .navigate(url)
        .map_err(|e| format!("Failed to navigate: {e}"))
}

fn gateway_logs(lines: Option<usize>) -> Result<String, String> {
    let (log_path, error_log_path) = gateway_log_paths()?;

    if !log_path.exists() {
        return Ok("No logs available yet. Start the gateway to see logs.".to_string());
    }

    let max_lines = lines.unwrap_or(100);
    let output = read_log_tail(&log_path, max_lines, LOG_TAIL_MAX_BYTES)
        .map_err(|e| format!("Failed to read log file: {e}"))?;
    let errors = if error_log_path.exists() {
        read_log_tail(&error_log_path, max_lines, LOG_TAIL_MAX_BYTES)
            .map_err(|e| format!("Failed to read error log file: {e}"))?
    } else {
        String::new()
    };

    Ok(compose_gateway_logs(&output, &errors))
}

/// Get the gateway logs from the log files
#[tauri::command]
async fn get_gateway_logs(lines: Option<usize>) -> Result<String, String> {
    run_blocking(move || gateway_logs(lines)).await?
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

fn tray_status_label(running: bool) -> &'static str {
    if running {
        "🟢 Running"
    } else {
        "🔴 Stopped"
    }
}

/// Build the tray menu. The status item is returned too so it can be updated later.
fn create_tray_menu<R: Runtime>(
    app: &tauri::AppHandle<R>,
) -> tauri::Result<(Menu<R>, MenuItem<R>)> {
    // Starts as "stopped"; the status monitor corrects it within moments without probing here
    let status_item =
        MenuItem::with_id(app, "status", tray_status_label(false), false, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &status_item,
            &MenuItem::with_id(app, "separator", "─────────", false, None::<&str>)?,
            &MenuItem::with_id(app, "start", "▶ Start Gateway", true, None::<&str>)?,
            &MenuItem::with_id(app, "stop", "⏹ Stop Gateway", true, None::<&str>)?,
            &MenuItem::with_id(app, "dashboard", "🌐 Open Dashboard", true, None::<&str>)?,
            &MenuItem::with_id(app, "separator2", "─────────", false, None::<&str>)?,
            &MenuItem::with_id(app, "quit", "✖ Quit", true, None::<&str>)?,
        ],
    )?;

    Ok((menu, status_item))
}

/// Keep the tray status line in sync with the gateway, and reap the gateway process once it exits
fn spawn_tray_status_monitor<R: Runtime>(status_item: MenuItem<R>) {
    std::thread::spawn(move || {
        let mut shown = false;
        loop {
            child_still_running(&mut lock_gateway_child());

            let running = is_gateway_running();
            if running != shown {
                match status_item.set_text(tray_status_label(running)) {
                    Ok(()) => shown = running,
                    Err(e) => eprintln!("Failed to update tray status: {e}"),
                }
            }
            std::thread::sleep(TRAY_STATUS_INTERVAL);
        }
    });
}

/// Tray actions spawn processes, so they run off the main thread. Failures are logged, not dropped.
fn run_tray_action<F>(name: &'static str, action: F)
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    std::thread::spawn(move || {
        if let Err(e) = action() {
            eprintln!("Tray action '{name}' failed: {e}");
        }
    });
}

fn show_main_window<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(not(target_os = "windows"))]
    augment_process_path();

    tauri::Builder::default()
        // Must be the first plugin: it has to see a second launch before anything else runs
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Focus existing window instead of opening a duplicate
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // Create system tray
            let (menu, status_item) = create_tray_menu(app.handle())?;

            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "start" => run_tray_action("start", start_gateway_blocking),
                    "stop" => run_tray_action("stop", || run_openclaw_gateway_control("stop")),
                    "dashboard" => show_main_window(app),
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
                        show_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            let _tray = tray.build(app)?;

            spawn_tray_status_monitor(status_item);

            // Handle window close - minimize to tray instead of quitting
            if let Some(main_window) = app.get_webview_window("main") {
                let main_window_clone = main_window.clone();
                main_window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        // Prevent the window from closing
                        api.prevent_close();
                        // Hide the window instead
                        let _ = main_window_clone.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_gateway_status,
            start_gateway,
            stop_gateway,
            restart_gateway,
            auto_start_gateway,
            get_dashboard_url,
            open_dashboard_in_browser,
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
    use std::io::Write;
    use std::time::Instant;

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "openclaw-desktop-test-{}-{name}",
            std::process::id()
        ))
    }

    fn program_and_args(command: &Command) -> (String, Vec<String>) {
        (
            command.get_program().to_string_lossy().into_owned(),
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
        )
    }

    /// A command that exits immediately on every platform
    fn quick_exit_command() -> Command {
        #[cfg(target_os = "windows")]
        {
            let mut command = hidden_command("cmd");
            command.args(["/c", "exit", "0"]);
            command
        }
        #[cfg(not(target_os = "windows"))]
        {
            hidden_command("true")
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn npm_and_openclaw_go_through_cmd_on_windows() {
        // `npm` and `openclaw` are `.cmd` shims there, which `Command` cannot resolve by bare name
        let (program, args) = program_and_args(&npm_command());
        assert_eq!(program, "cmd");
        assert_eq!(args, ["/c", "npm"]);

        let (program, args) = program_and_args(&openclaw_command());
        assert_eq!(program, "cmd");
        assert_eq!(args, ["/c", "openclaw"]);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn npm_and_openclaw_run_directly_off_windows() {
        let (program, args) = program_and_args(&npm_command());
        assert_eq!(program, "npm");
        assert!(args.is_empty());

        let (program, args) = program_and_args(&openclaw_command());
        assert_eq!(program, "openclaw");
        assert!(args.is_empty());
    }

    #[test]
    fn is_port_open_detects_a_listener_and_gives_up_quickly_on_a_closed_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let port = listener.local_addr().expect("listener address").port();
        assert!(is_port_open(port, Duration::from_millis(500)));

        drop(listener);
        let started = Instant::now();
        assert!(!is_port_open(port, Duration::from_millis(500)));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn run_blocking_moves_work_off_the_calling_thread() {
        let caller = std::thread::current().id();
        let worker = tauri::async_runtime::block_on(run_blocking(|| std::thread::current().id()));
        assert_eq!(worker.map(|id| id != caller), Ok(true));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn merge_path_dirs_appends_existing_dirs_once_and_keeps_order() {
        let current = vec![PathBuf::from("/usr/bin"), PathBuf::from("/usr/local/bin")];
        let candidates = vec![
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/missing/bin"),
            PathBuf::from("/opt/homebrew/bin"),
        ];
        let merged = merge_path_dirs(current, candidates, |dir| dir != Path::new("/missing/bin"));
        assert_eq!(
            merged,
            [
                PathBuf::from("/usr/bin"),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/opt/homebrew/bin"),
            ]
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn node_versions_sort_numerically_newest_first() {
        let names = vec![
            "v9.11.2".to_string(),
            "v22.3.0".to_string(),
            "v18.20.4".to_string(),
        ];
        assert_eq!(
            sort_node_versions_newest_first(names),
            ["v22.3.0", "v18.20.4", "v9.11.2"]
        );
    }

    #[test]
    fn read_log_tail_tolerates_invalid_utf8() {
        let path = temp_file("invalid-utf8.log");
        fs::write(&path, b"ok\n\xff\xfe broken\nlast\n").expect("write test log");

        let tail = read_log_tail(&path, 10, 1024).expect("read tail");
        assert!(tail.starts_with("ok\n"));
        assert!(tail.ends_with("last"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn read_log_tail_only_reads_the_end_of_a_large_file() {
        let path = temp_file("large.log");
        let lines: Vec<String> = (0..1000).map(|i| format!("line {i}")).collect();
        fs::write(&path, lines.join("\n") + "\n").expect("write test log");

        // 64 bytes hold only the last few lines, and the cut lands in the middle of one
        let tail = read_log_tail(&path, 200, 64).expect("read tail");
        let lines: Vec<&str> = tail.lines().collect();
        assert_eq!(lines.last(), Some(&"line 999"));
        assert!(lines.len() < 10);
        assert!(
            lines.iter().all(|line| line.starts_with("line ")),
            "partial first line leaked: {lines:?}"
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn compose_gateway_logs_is_just_the_output_when_there_are_no_errors() {
        assert_eq!(compose_gateway_logs("started", ""), "started");
        assert_eq!(compose_gateway_logs("started", "  \n"), "started");
    }

    #[test]
    fn compose_gateway_logs_appends_the_error_log() {
        assert_eq!(
            compose_gateway_logs("started", "EADDRINUSE"),
            "started\n\n===== gateway_error.log =====\nEADDRINUSE"
        );
        // A gateway that died before printing anything still shows why
        assert_eq!(
            compose_gateway_logs("", "config missing"),
            "===== gateway_error.log =====\nconfig missing"
        );
    }

    #[test]
    fn child_still_running_is_false_for_an_empty_slot() {
        let mut slot = None;
        assert!(!child_still_running(&mut slot));
    }

    #[test]
    fn child_still_running_reaps_an_exited_child() {
        let child = quick_exit_command()
            .stdout(Stdio::null())
            .spawn()
            .expect("spawn test child");
        let mut slot = Some(child);

        let deadline = Instant::now() + Duration::from_secs(10);
        while child_still_running(&mut slot) {
            assert!(Instant::now() < deadline, "test child did not exit in time");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(slot.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn child_still_running_is_true_for_a_live_child() {
        let child = hidden_command("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let mut slot = Some(child);
        assert!(child_still_running(&mut slot));

        if let Some(mut child) = slot.take() {
            child.kill().expect("kill sleep");
            child.wait().expect("reap sleep");
        }
    }

    #[test]
    fn open_fresh_log_keeps_writing_at_the_end_after_the_log_is_cleared() {
        let path = temp_file("append.log");
        let mut handle = open_fresh_log(&path).expect("open log");
        handle.write_all(b"first\n").expect("write first line");

        // What clear_gateway_logs does while the gateway still holds its handle
        fs::write(&path, "").expect("clear log");
        handle.write_all(b"second\n").expect("write second line");

        assert_eq!(fs::read(&path).expect("read log"), b"second\n");
        let _ = fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn open_fresh_log_is_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_file("private.log");
        let _handle = open_fresh_log(&path).expect("open log");
        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn tray_status_label_reflects_the_gateway_state() {
        assert_eq!(tray_status_label(true), "🟢 Running");
        assert_eq!(tray_status_label(false), "🔴 Stopped");
    }

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
        assert_eq!(read_token_from_file(path), None);
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
