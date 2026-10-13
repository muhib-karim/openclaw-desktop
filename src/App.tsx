import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  appendDoctorOutput,
  describeOpenClawInstall,
  gatewayStatusLabel,
  getGatewayState,
} from "./lib/gateway";
import "./App.css";

interface GatewayStatus {
  running: boolean;
  port: number;
  dashboard_url: string;
}

interface GatewayDiagnostics {
  openclaw_installed: boolean;
  gateway_running: boolean;
  gateway_port: number;
  dashboard_url: string;
  openclaw_version: string | null;
  profile_name: string | null;
  log_path: string;
  error_log_path: string;
}

type Page = "loading" | "setup" | "dashboard";

function App() {
  const [page, setPage] = useState<Page>("loading");
  const [status, setStatus] = useState<GatewayStatus | null>(null);
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [startingGateway, setStartingGateway] = useState(false);
  const [navigatedToDashboard, setNavigatedToDashboard] = useState(false);

  // Log panel state
  const [showLogs, setShowLogs] = useState(false);
  const [logs, setLogs] = useState<string>("");
  const [diagnostics, setDiagnostics] = useState<GatewayDiagnostics | null>(null);
  const [runningDoctor, setRunningDoctor] = useState(false);

  // Auto-start gateway on first load
  useEffect(() => {
    async function init() {
      const isInstalled = await invoke<boolean>("is_openclaw_installed");
      if (!isInstalled) {
        setPage("setup");
        return;
      }

      setPage("dashboard");

      // Auto-start if not running
      try {
        const started = await invoke<boolean>("auto_start_gateway");
        if (started) {
          setStartingGateway(true);
        }
      } catch (e) {
        console.error("Auto-start failed:", e);
      }
    }
    init();
  }, []);

  // Poll status every 3 seconds
  useEffect(() => {
    if (page !== "dashboard") return;

    async function checkStatus() {
      try {
        const gatewayStatus = await invoke<GatewayStatus>("get_gateway_status");
        setStatus(gatewayStatus);

        if (gatewayStatus.running) {
          setStartingGateway(false);

          // Navigate the entire webview to the dashboard (bypasses X-Frame-Options)
          if (!navigatedToDashboard) {
            setNavigatedToDashboard(true);
            try {
              await invoke("open_dashboard_window");
            } catch (e) {
              console.error("Failed to navigate to dashboard:", e);
            }
          }
        } else {
          // Gateway stopped — reset so we re-navigate when it comes back
          if (navigatedToDashboard) {
            setNavigatedToDashboard(false);
          }
        }
      } catch (e) {
        console.error("Status check failed:", e);
      }
    }

    checkStatus();
    const interval = setInterval(checkStatus, 3000);
    return () => clearInterval(interval);
  }, [page, navigatedToDashboard]);

  // Poll logs when panel is open
  useEffect(() => {
    if (!showLogs) return;

    async function fetchLogs() {
      try {
        const logContent = await invoke<string>("get_gateway_logs", { lines: 200 });
        setLogs(logContent);
      } catch (e) {
        console.error("Failed to fetch logs:", e);
      }
    }

    fetchLogs();
    fetchDiagnostics();
    const interval = setInterval(fetchLogs, 1000);
    return () => clearInterval(interval);
  }, [showLogs]);

  async function fetchDiagnostics() {
    try {
      const data = await invoke<GatewayDiagnostics>("get_gateway_diagnostics");
      setDiagnostics(data);
    } catch (e) {
      console.error("Failed to fetch diagnostics:", e);
    }
  }

  async function runDoctor() {
    if (runningDoctor) return;
    setRunningDoctor(true);
    try {
      const output = await invoke<string>("run_openclaw_doctor");
      setLogs((prev) => appendDoctorOutput(prev, output, false));
    } catch (e) {
      console.error("OpenClaw doctor failed:", e);
      setLogs((prev) => appendDoctorOutput(prev, String(e), true));
    } finally {
      setRunningDoctor(false);
      fetchDiagnostics();
    }
  }

  async function handleInstall() {
    setInstalling(true);
    setError(null);
    try {
      await invoke("install_openclaw");
      setPage("dashboard");
      // Auto-start after install
      try {
        await invoke<boolean>("auto_start_gateway");
        setStartingGateway(true);
      } catch (e) {
        console.error("Auto-start after install failed:", e);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setInstalling(false);
    }
  }

  async function handleStartGateway() {
    if (startingGateway) return;
    setStartingGateway(true);
    try {
      await invoke("start_gateway");
    } catch (e) {
      console.error("Failed to start gateway:", e);
      setStartingGateway(false);
    }
  }

  async function handleRestartGateway() {
    try {
      await invoke("restart_gateway");
      setNavigatedToDashboard(false);
      setStartingGateway(true);
    } catch (e) {
      console.error("Failed to restart gateway:", e);
    }
  }

  async function openInBrowser() {
    const url = await invoke<string>("get_dashboard_url");
    await openUrl(url);
  }

  async function openKofi() {
    await openUrl("https://ko-fi.com/ai_dev_2024");
  }

  const gatewayState = getGatewayState(status?.running, startingGateway);

  // Loading screen
  if (page === "loading") {
    return (
      <main className="container loading">
        <div className="logo-container">
          <span className="logo-emoji">🦞</span>
        </div>
        <h1>OpenClaw Desktop</h1>
        <p className="loading-text">Checking installation...</p>
      </main>
    );
  }

  // Setup wizard
  if (page === "setup") {
    return (
      <main className="container setup">
        <div className="logo-container">
          <span className="logo-emoji">🦞</span>
        </div>
        <h1>Welcome to OpenClaw Desktop</h1>
        <p className="subtitle">Your AI assistant, now with a beautiful UI</p>

        <div className="setup-card">
          <h2>🚀 Quick Setup</h2>
          <p>OpenClaw needs to be installed on your system. This will:</p>
          <ul>
            <li>Install OpenClaw via npm (requires Node.js)</li>
            <li>Set up the gateway service</li>
            <li>Get you ready to use your AI assistant</li>
          </ul>

          {error && <p className="error">{error}</p>}

          <button
            className="primary-btn"
            onClick={handleInstall}
            disabled={installing}
          >
            {installing ? "Installing..." : "Install OpenClaw"}
          </button>
        </div>

        <footer className="setup-footer">
          <button className="support-btn" onClick={openKofi}>
            ☕ Support Development
          </button>
          <p className="credits">Made with ❤️ by ai-dev-2024</p>
        </footer>
      </main>
    );
  }

  // Dashboard view — when gateway starts, this navigates to the dashboard automatically
  // The React UI is only shown while gateway is stopped or starting
  return (
    <main className="container dashboard">
      {/* Header Bar (visible until dashboard takes over) */}
      <header className="status-bar">
        <div className="status-left">
          <span className="logo-small">🦞</span>
          <span className="app-title">OpenClaw Desktop</span>
          {diagnostics?.openclaw_version && (
            <span className="version-badge">v{diagnostics.openclaw_version}</span>
          )}
        </div>

        <div className="status-center">
          <span className={`status-indicator ${gatewayState}`}>
            {gatewayStatusLabel(gatewayState)}
          </span>
        </div>

        <div className="status-right">
          {!status?.running ? (
            <button
              className="control-btn start"
              onClick={handleStartGateway}
              disabled={startingGateway}
            >
              {startingGateway ? "Starting..." : "▶ Start"}
            </button>
          ) : (
            <>
              <button className="control-btn restart" onClick={handleRestartGateway} title="Restart Gateway">
                🔄 Restart
              </button>
              <button
                className="control-btn icon-btn"
                onClick={openInBrowser}
                title="Open in Browser"
              >
                🔗
              </button>
            </>
          )}
          <button
            className={`control-btn logs-btn ${showLogs ? "active" : ""}`}
            onClick={() => setShowLogs(!showLogs)}
            title={showLogs ? "Hide Logs" : "Show Logs"}
          >
            📋 {showLogs ? "Hide Logs" : "Logs"}
          </button>
          <button className="support-btn-small" onClick={openKofi} title="Support on Ko-fi">
            ☕
          </button>
        </div>
      </header>

      {/* Gateway Offline / Starting view */}
      {!status?.running && (
        <div className="gateway-offline">
          <span className="offline-emoji">🦞</span>
          <h2>{startingGateway ? "Starting Gateway..." : "Gateway is not running"}</h2>
          <p>{startingGateway ? "Please wait, this may take a few seconds..." : "Click \"Start\" to launch the OpenClaw gateway"}</p>
          {!startingGateway && (
            <button className="primary-btn" onClick={handleStartGateway}>
              ▶ Start Gateway
            </button>
          )}

          <div className="support-section">
            <p>Enjoying OpenClaw Desktop?</p>
            <button className="support-btn" onClick={openKofi}>
              ☕ Buy me a coffee on Ko-fi
            </button>
          </div>
        </div>
      )}

      {/* When running, show a brief "navigating..." message before dashboard takes over */}
      {status?.running && !navigatedToDashboard && (
        <div className="gateway-offline">
          <span className="loading-spinner">🦞</span>
          <p className="loading-text">Loading Dashboard...</p>
        </div>
      )}

      {/* Log Panel */}
      {showLogs && (
        <div className="log-panel">
          <div className="log-header">
            <span>📋 Gateway Logs</span>
            <div className="log-actions">
              <button className="log-tool-btn" onClick={fetchDiagnostics} title="Refresh diagnostics">
                Refresh
              </button>
              <button className="log-tool-btn" onClick={runDoctor} disabled={runningDoctor} title="Run openclaw doctor">
                {runningDoctor ? "Running..." : "Run Doctor"}
              </button>
              <button
                className="log-close-btn"
                onClick={() => setShowLogs(false)}
                title="Close logs"
              >
                ✕
              </button>
            </div>
          </div>
          {diagnostics && (
            <div className="diagnostics-row">
              <span>OpenClaw: {describeOpenClawInstall(diagnostics.openclaw_installed, diagnostics.openclaw_version)}</span>
              <span>Gateway: {diagnostics.gateway_running ? "Running" : "Stopped"}</span>
              <span>Port: {diagnostics.gateway_port}</span>
            </div>
          )}
          <pre className="log-content">{logs || "No logs available yet..."}</pre>
        </div>
      )}
    </main>
  );
}

export default App;
