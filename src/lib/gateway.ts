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

export interface DoctorOutput {
  text: string;
  failed: boolean;
}

/** Text for the log panel: the gateway log followed by the latest doctor run, if there is one. */
export function composeLogView(logs: string, doctor: DoctorOutput | null): string {
  if (!doctor) return logs;
  return appendDoctorOutput(logs, doctor.text, doctor.failed);
}

/** True once a requested gateway start has been pending for `timeoutMs` or longer. */
export function hasStartTimedOut(startRequestedAt: number | null, now: number, timeoutMs: number): boolean {
  if (startRequestedAt === null) return false;
  return now - startRequestedAt >= timeoutMs;
}

/** Turn whatever an `invoke` call rejected with into a message that can be shown to the user. */
export function toErrorMessage(error: unknown): string {
  if (typeof error === "string" && error.trim()) return error.trim();
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  return "Unknown error";
}
