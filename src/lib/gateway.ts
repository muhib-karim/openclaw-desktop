export type GatewayState = "running" | "starting" | "stopped";

export function getGatewayState(running: boolean | undefined, starting: boolean): GatewayState {
  if (running) return "running";
  return starting ? "starting" : "stopped";
}

export function gatewayStatusLabel(state: GatewayState): string {
  switch (state) {
    case "running":
      return "🟢 Gateway Running";
    case "starting":
      return "🟡 Starting...";
    case "stopped":
      return "🔴 Gateway Stopped";
  }
}

export function appendDoctorOutput(previous: string, output: string, failed: boolean): string {
  const heading = failed ? "===== openclaw doctor (failed) =====" : "===== openclaw doctor =====";
  return `${previous}\n\n${heading}\n${output}`.trim();
}

export function describeOpenClawInstall(installed: boolean, version: string | null): string {
  if (!installed) return "Not installed";
  return version || "Installed";
}
