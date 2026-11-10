// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

type Handlers = Record<string, () => unknown>;

/** Route `invoke(cmd)` to a handler; a handler that throws becomes a rejected command. */
function mockBackend(handlers: Handlers) {
  invokeMock.mockImplementation((cmd: string) => {
    const handler = handlers[cmd];
    if (!handler) return Promise.reject(new Error(`unexpected command: ${cmd}`));
    try {
      return Promise.resolve(handler());
    } catch (e) {
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- Tauri rejects with strings
      return Promise.reject(e);
    }
  });
}

function rejectWith(message: string): () => never {
  return () => {
    // eslint-disable-next-line @typescript-eslint/only-throw-error -- Tauri rejects with strings
    throw message;
  };
}

function calls(cmd: string): number {
  return invokeMock.mock.calls.filter(([name]) => name === cmd).length;
}

const stopped = { running: false, port: 18789, dashboard_url: "http://127.0.0.1:18789/" };
const running = { ...stopped, running: true };
const diagnostics = {
  openclaw_installed: true,
  gateway_running: false,
  gateway_port: 18789,
  dashboard_url: "http://127.0.0.1:18789/",
  openclaw_version: "2026.2.1",
  profile_name: null,
  log_path: "/home/me/.openclaw/gateway.log",
  error_log_path: "/home/me/.openclaw/gateway_error.log",
};

beforeEach(() => {
  invokeMock.mockReset();
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("App", () => {
  it("shows an error with a retry instead of hanging when the install check fails", async () => {
    mockBackend({ is_openclaw_installed: rejectWith("ipc unavailable") });
    render(<App />);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("ipc unavailable");
    expect(screen.queryByText("Checking installation...")).toBeNull();

    mockBackend({ is_openclaw_installed: () => false });
    fireEvent.click(screen.getByText("Try again"));
    expect(await screen.findByText("Welcome to OpenClaw Desktop")).toBeTruthy();
  });

  it("keeps the doctor output visible after the next log poll", async () => {
    mockBackend({
      is_openclaw_installed: () => true,
      auto_start_gateway: () => false,
      get_gateway_status: () => stopped,
      get_gateway_logs: () => "gateway line",
      get_gateway_diagnostics: () => diagnostics,
      run_openclaw_doctor: () => "doctor says all good",
    });
    render(<App />);

    fireEvent.click(await screen.findByTitle("Show Logs"));
    fireEvent.click(await screen.findByText("Run Doctor"));
    await screen.findByText(/doctor says all good/);

    const pollsBefore = calls("get_gateway_logs");
    await waitFor(() => expect(calls("get_gateway_logs")).toBeGreaterThan(pollsBefore), {
      timeout: 3000,
    });
    const logPanel = screen.getByText(/gateway line/);
    expect(logPanel.textContent).toContain("doctor says all good");
  });

  it("reports a failed dashboard navigation and retries on the next poll", async () => {
    mockBackend({
      is_openclaw_installed: () => true,
      auto_start_gateway: () => false,
      get_gateway_status: () => running,
      open_dashboard_window: rejectWith("Failed to navigate: webview gone"),
    });
    render(<App />);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Could not open the dashboard: Failed to navigate: webview gone");
    // Not a blank page: the loading view is back, so the next poll can try again
    expect(screen.getByText("Loading Dashboard...")).toBeTruthy();

    await waitFor(() => expect(calls("open_dashboard_window")).toBeGreaterThan(1), { timeout: 5000 });
  }, 8000);

  it("navigates to the dashboard only once while the gateway keeps running", async () => {
    mockBackend({
      is_openclaw_installed: () => true,
      auto_start_gateway: () => false,
      get_gateway_status: () => running,
      open_dashboard_window: () => null,
    });
    render(<App />);

    await waitFor(() => expect(calls("get_gateway_status")).toBeGreaterThan(1), { timeout: 5000 });
    expect(calls("open_dashboard_window")).toBe(1);
  }, 8000);

  it("shows why a start failed and lets the user try again", async () => {
    mockBackend({
      is_openclaw_installed: () => true,
      auto_start_gateway: () => false,
      get_gateway_status: () => stopped,
      start_gateway: rejectWith("Failed to start gateway: program not found"),
    });
    render(<App />);

    fireEvent.click(await screen.findByText("▶ Start Gateway"));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Could not start the gateway: Failed to start gateway: program not found");
    expect(screen.getByText("▶ Start Gateway")).toBeTruthy();
    expect(screen.getByText<HTMLButtonElement>("▶ Start").disabled).toBe(false);
  });

  it("gives up on a start that never opens the port", async () => {
    vi.useFakeTimers();
    mockBackend({
      is_openclaw_installed: () => true,
      auto_start_gateway: () => true,
      get_gateway_status: () => stopped,
    });
    render(<App />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });
    expect(screen.getByText("Starting Gateway...")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(33_000);
    });
    expect(screen.getByRole("alert").textContent).toContain("did not start listening on port 18789");
    expect(screen.getByText("Gateway is not running")).toBeTruthy();
    expect(screen.getByText<HTMLButtonElement>("▶ Start").disabled).toBe(false);
  });
});
