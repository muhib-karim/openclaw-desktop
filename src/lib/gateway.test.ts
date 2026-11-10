import { describe, expect, it } from "vitest";
import {
  appendDoctorOutput,
  composeLogView,
  describeOpenClawInstall,
  gatewayStatusLabel,
  getGatewayState,
  hasStartTimedOut,
  toErrorMessage,
} from "./gateway";

describe("getGatewayState", () => {
  it("reports running whenever the gateway is up", () => {
    expect(getGatewayState(true, false)).toBe("running");
    expect(getGatewayState(true, true)).toBe("running");
  });

  it("reports starting while a start is pending", () => {
    expect(getGatewayState(false, true)).toBe("starting");
    expect(getGatewayState(undefined, true)).toBe("starting");
  });

  it("reports stopped when not running and not starting", () => {
    expect(getGatewayState(false, false)).toBe("stopped");
    expect(getGatewayState(undefined, false)).toBe("stopped");
  });
});

describe("gatewayStatusLabel", () => {
  it("maps each state to a distinct label", () => {
    expect(gatewayStatusLabel("running")).toBe("🟢 Gateway Running");
    expect(gatewayStatusLabel("starting")).toBe("🟡 Starting...");
    expect(gatewayStatusLabel("stopped")).toBe("🔴 Gateway Stopped");
  });
});

describe("appendDoctorOutput", () => {
  it("appends a success section after existing logs", () => {
    expect(appendDoctorOutput("line 1", "all good", false)).toBe(
      "line 1\n\n===== openclaw doctor =====\nall good",
    );
  });

  it("marks failed runs and trims leading whitespace when logs are empty", () => {
    expect(appendDoctorOutput("", "boom", true)).toBe("===== openclaw doctor (failed) =====\nboom");
  });
});

describe("describeOpenClawInstall", () => {
  it("shows the detected version when installed", () => {
    expect(describeOpenClawInstall(true, "2026.2.1")).toBe("2026.2.1");
  });

  it("falls back to a generic label when the version is unknown", () => {
    expect(describeOpenClawInstall(true, null)).toBe("Installed");
  });

  it("reports missing installs", () => {
    expect(describeOpenClawInstall(false, "2026.2.1")).toBe("Not installed");
  });
});

describe("composeLogView", () => {
  it("shows the logs unchanged when doctor has not run", () => {
    expect(composeLogView("line 1", null)).toBe("line 1");
    expect(composeLogView("", null)).toBe("");
  });

  it("keeps the doctor section when the logs are refreshed", () => {
    const doctor = { text: "all good", failed: false };
    expect(composeLogView("old logs", doctor)).toContain("===== openclaw doctor =====\nall good");
    // A later poll replaces the logs; the doctor output must still be there
    expect(composeLogView("new logs", doctor)).toBe("new logs\n\n===== openclaw doctor =====\nall good");
  });

  it("marks a failed doctor run", () => {
    expect(composeLogView("", { text: "boom", failed: true })).toBe(
      "===== openclaw doctor (failed) =====\nboom",
    );
  });
});

describe("hasStartTimedOut", () => {
  it("never times out when no start is pending", () => {
    expect(hasStartTimedOut(null, 1_000_000, 30_000)).toBe(false);
  });

  it("stays pending before the deadline", () => {
    expect(hasStartTimedOut(1_000, 30_999, 30_000)).toBe(false);
  });

  it("times out at and after the deadline", () => {
    expect(hasStartTimedOut(1_000, 31_000, 30_000)).toBe(true);
    expect(hasStartTimedOut(1_000, 90_000, 30_000)).toBe(true);
  });
});

describe("toErrorMessage", () => {
  it("passes through the string errors Tauri commands reject with", () => {
    expect(toErrorMessage("  Failed to start gateway: not found \n")).toBe(
      "Failed to start gateway: not found",
    );
  });

  it("uses the message of Error objects", () => {
    expect(toErrorMessage(new Error("ipc unavailable"))).toBe("ipc unavailable");
  });

  it("falls back to a generic message for anything else", () => {
    expect(toErrorMessage(undefined)).toBe("Unknown error");
    expect(toErrorMessage("")).toBe("Unknown error");
    expect(toErrorMessage({ code: 1 })).toBe("Unknown error");
  });
});
