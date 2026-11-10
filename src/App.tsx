import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  composeLogView,
  describeOpenClawInstall,
  gatewayStatusLabel,
  getGatewayState,
  hasStartTimedOut,
  toErrorMessage,
  type DoctorOutput,
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

type Page = "loading" | "setup" | "dashboard" | "error";

const STATUS_POLL_MS = 3000;
const LOG_POLL_MS = 1000;
const START_TIMEOUT_MS = 30_000;
const START_TIMEOUT_MESSAGE =
  "The gateway did not start listening on port 18789 within 30 seconds. Open Logs to see why.";

/** Diagnostics are best-effort: a failure hides the row instead of breaking the log panel. */
async function loadDiagnostics(): Promise<GatewayDiagnostics | null> {
  try {
    return await invoke<GatewayDiagnostics>("get_gateway_diagnostics");
  } catch (e) {
    console.error("Failed to fetch diagnostics:", e);
    return null;
  }
}

function App() {
  const [page, setPage] = useState<Page>("loading");
  const [status, setStatus] = useState<GatewayStatus | null>(null);
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [startingGateway, setStartingGateway] = useState(false);
  const [navigatedToDashboard, setNavigatedToDashboard] = useState(false);

  // When the pending start was requested; lets the status poll give up on a start that never lands
  const startRequestedAt = useRef<number | null>(null);
  // True from the moment a dashboard navigation is requested, so overlapping polls cannot repeat it
  const navigating = useRef(false);

  // Log panel state
  const [showLogs, setShowLogs] = useState(false);
  const [logs, setLogs] = useState<string>("");
  const [doctorOutput, setDoctorOutput] = useState<DoctorOutput | null>(null);
  const [diagnostics, setDiagnostics] = useState<GatewayDiagnostics | null>(null);
  const [runningDoctor, setRunningDoctor] = useState(false);

  const markStarting = useCallback(() => {
    startRequestedAt.current = Date.now();
    setStartingGateway(true);
  }, []);

  const clearStarting = useCallback(() => {
    startRequestedAt.current = null;
    setStartingGateway(false);
  }, []);

  // Bumped by "Try again" to re-run the install check
  const [initAttempt, setInitAttempt] = useState(0);

  // Check the installation, then auto-start the gateway
  useEffect(() => {
    let cancelled = false;

    async function init() {
      try {
        const isInstalled = await invoke<boolean>("is_openclaw_installed");
        if (cancelled) return;
        if (!isInstalled) {
          setPage("setup");
          return;
        }
      } catch (e) {
        console.error("Install check failed:", e);
        if (cancelled) return;
        setError(toErrorMessage(e));
        setPage("error");
        return;
      }

      setPage("dashboard");

      // Auto-start if not running
      try {
        const started = await invoke<boolean>("auto_start_gateway");
        if (!cancelled && started) {
          markStarting();
        }
      } catch (e) {
        console.error("Auto-start failed:", e);
        if (!cancelled) {
          setError(`Could not start the gateway: ${toErrorMessage(e)}`);
        }
      }
    }

    void init();
    return () => {
      cancelled = true;
    };
  }, [initAttempt, markStarting]);

  // Poll status every 3 seconds
  useEffect(() => {
    if (page !== "dashboard") return;

    let cancelled = false;
    let inFlight = false;

    async function checkStatus() {
      // A slow backend must not stack up overlapping checks
      if (inFlight) return;
      inFlight = true;
      try {
        const gatewayStatus = await invoke<GatewayStatus>("get_gateway_status");
        if (cancelled) return;
        setStatus(gatewayStatus);

        if (!gatewayStatus.running) {
          // Gateway is down: allow a fresh navigation when it comes back
          navigating.current = false;
          setNavigatedToDashboard(false);
          if (hasStartTimedOut(startRequestedAt.current, Date.now(), START_TIMEOUT_MS)) {
            clearStarting();
            setError(START_TIMEOUT_MESSAGE);
          }
          return;
        }

        clearStarting();
        if (navigating.current) return;

        // Navigate the entire webview to the dashboard (bypasses X-Frame-Options)
        navigating.current = true;
        setNavigatedToDashboard(true);
        try {
          await invoke("open_dashboard_window");
        } catch (e) {
          console.error("Failed to navigate to dashboard:", e);
          // Let the next poll retry instead of leaving a blank page
          navigating.current = false;
          if (!cancelled) {
            setNavigatedToDashboard(false);
            setError(`Could not open the dashboard: ${toErrorMessage(e)}`);
          }
        }
      } catch (e) {
        console.error("Status check failed:", e);
        if (!cancelled) {
          setError(`Could not read the gateway status: ${toErrorMessage(e)}`);
        }
      } finally {
        inFlight = false;
      }
    }

    void checkStatus();
    const interval = setInterval(() => void checkStatus(), STATUS_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [page, clearStarting]);

  // Poll logs when panel is open
  useEffect(() => {
    if (!showLogs) return;

    let cancelled = false;
    let inFlight = false;

    async function fetchLogs() {
      if (inFlight) return;
      inFlight = true;
      try {
        const logContent = await invoke<string>("get_gateway_logs", { lines: 200 });
        if (!cancelled) setLogs(logContent);
      } catch (e) {
        console.error("Failed to fetch logs:", e);
        if (!cancelled) setLogs(`Could not read the gateway log: ${toErrorMessage(e)}`);
      } finally {
        inFlight = false;
      }
    }

    void fetchLogs();
    void loadDiagnostics().then((data) => {
      if (!cancelled) setDiagnostics(data);
    });
    const interval = setInterval(() => void fetchLogs(), LOG_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [showLogs]);

  async function refreshDiagnostics() {
    setDiagnostics(await loadDiagnostics());
  }

  async function runDoctor() {
    if (runningDoctor) return;
    setRunningDoctor(true);
    try {
      const output = await invoke<string>("run_openclaw_doctor");
      // Kept apart from `logs`, which the log poll replaces every second
      setDoctorOutput({ text: output, failed: false });
    } catch (e) {
      console.error("OpenClaw doctor failed:", e);
      setDoctorOutput({ text: toErrorMessage(e), failed: true });
    } finally {
      setRunningDoctor(false);
      void refreshDiagnostics();
    }
  }

  function handleRetry() {
    setError(null);
    setPage("loading");
    setInitAttempt((attempt) => attempt + 1);
  }

  async function handleInstall() {
    setInstalling(true);
    setError(null);
    try {
      await invoke("install_openclaw");
    } catch (e) {
      console.error("Install failed:", e);
      setError(toErrorMessage(e));
      setInstalling(false);
      return;
    }

    setInstalling(false);
    setPage("dashboard");
    // Auto-start after install
    try {
      await invoke<boolean>("auto_start_gateway");
      markStarting();
    } catch (e) {
      console.error("Auto-start after install failed:", e);
      setError(`OpenClaw was installed, but the gateway could not be started: ${toErrorMessage(e)}`);
    }
  }

  async function handleStartGateway() {
    if (startingGateway) return;
    setError(null);
    markStarting();
    try {
      await invoke("start_gateway");
    } catch (e) {
      console.error("Failed to start gateway:", e);
      clearStarting();
      setError(`Could not start the gateway: ${toErrorMessage(e)}`);
    }
  }

  async function handleRestartGateway() {
    setError(null);
    try {
      await invoke("restart_gateway");
      navigating.current = false;
      setNavigatedToDashboard(false);
      markStarting();
    } catch (e) {
      console.error("Failed to restart gateway:", e);
      setError(`Could not restart the gateway: ${toErrorMessage(e)}`);
    }
  }

  async function openInBrowser() {
    try {
      // The backend builds the tokenized URL and opens it, so the token never reaches this page
      await invoke("open_dashboard_in_browser");
    } catch (e) {
      console.error("Failed to open the dashboard in a browser:", e);
      setError(`Could not open the dashboard in your browser: ${toErrorMessage(e)}`);
    }
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

  // The install check itself failed
  if (page === "error") {
    return (
      <main className="container loading">
        <div className="logo-container">
          <span className="logo-emoji">🦞</span>
        </div>
        <h1>OpenClaw Desktop</h1>
        <p className="error" role="alert">
          Could not check the OpenClaw installation: {error ?? "Unknown error"}
        </p>
        <button className="primary-btn" onClick={handleRetry}>
          Try again
        </button>
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

          {error && (
            <p className="error" role="alert">
              {error}
            </p>
          )}

          <button
            className="primary-btn"
            onClick={handleInstall}
            disabled={installing}
          >
            {installing ? "Installing..." : "Install OpenClaw"}
          </button>
        </div>

        <footer className="setup-footer">
          <p className="credits">Made with ❤️ by Muhib Karim</p>
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
        </div>
      </header>

      {error && (
        <div className="error-banner" role="alert">
          <span>{error}</span>
          <button className="error-dismiss" onClick={() => setError(null)} title="Dismiss">
            ✕
          </button>
        </div>
      )}

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
              <button className="log-tool-btn" onClick={refreshDiagnostics} title="Refresh diagnostics">
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
          <pre className="log-content">{composeLogView(logs, doctorOutput) || "No logs available yet..."}</pre>
        </div>
      )}
    </main>
  );
}

export default App;
