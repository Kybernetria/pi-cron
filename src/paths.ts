import { homedir } from "node:os";
import { join } from "node:path";

export function cronStateDir(): string {
  return process.env.PI_CRON_STATE_DIR || join(process.env.XDG_STATE_HOME || join(homedir(), ".local", "state"), "pi-cron");
}
export function cronRuntimeDir(): string {
  return process.env.PI_CRON_RUNTIME_DIR || (process.env.XDG_RUNTIME_DIR ? join(process.env.XDG_RUNTIME_DIR, "pi-cron") : join(cronStateDir(), "run"));
}
export function socketPath(): string { return process.env.PI_CRON_SOCKET || join(cronRuntimeDir(), "daemon.sock"); }
export function databasePath(): string { return process.env.PI_CRON_DB || join(cronStateDir(), "jobs.sqlite"); }
export function lockPath(): string { return join(cronRuntimeDir(), "daemon.lock"); }
