import { describe, expect, it } from "vitest";
import {
  appendDoctorOutput,
  describeOpenClawInstall,
  gatewayStatusLabel,
  getGatewayState,
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
