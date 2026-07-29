import { randomUUID } from "node:crypto";
import { access } from "node:fs/promises";
import { constants } from "node:fs";
import { createConnection } from "node:net";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import { socketPath } from "./paths.ts";
import type { DaemonMethod, DaemonRequest, DaemonResponse } from "./types.ts";

const MAX_RESPONSE_BYTES = 2_097_152;

export class CronClient {
  constructor(private readonly path = socketPath(), private readonly autoStart = true) {}

  async request<T = unknown>(method: DaemonMethod, params: unknown = {}): Promise<T> {
    try { return await this.send<T>(method, params); }
    catch (error) {
      if (!this.autoStart || !isUnavailable(error)) throw error;
      await this.startDaemon();
      return this.send<T>(method, params);
    }
  }

  private send<T>(method: DaemonMethod, params: unknown): Promise<T> {
    const request: DaemonRequest = { id: randomUUID(), method, params };
    return new Promise<T>((resolve, reject) => {
      const socket = createConnection(this.path);
      let data = "";
      socket.setEncoding("utf8");
      socket.setTimeout(requestTimeout(method), () => socket.destroy(new Error(`daemon ${method} request timed out`)));
      socket.once("connect", () => socket.write(`${JSON.stringify(request)}\n`));
      socket.on("data", (chunk: string) => {
        data += chunk;
        if (Buffer.byteLength(data) > MAX_RESPONSE_BYTES) socket.destroy(new Error("daemon response too large"));
      });
      socket.once("error", reject);
      socket.once("end", () => {
        try {
          const response = JSON.parse(data.trim()) as DaemonResponse;
          if (!response.ok) reject(new Error(response.error.message));
          else resolve(response.result as T);
        } catch (error) { reject(error); }
      });
    });
  }

  private async startDaemon(): Promise<void> {
    const executable = process.env.PI_CRON_DAEMON_BIN || fileURLToPath(new URL("../rust/target/release/pi-cron-daemon", import.meta.url));
    try { await access(executable, constants.X_OK); }
    catch (error) { throw new Error(`pi-cron daemon executable is unavailable at ${executable}; run npm run build:daemon or set PI_CRON_DAEMON_BIN: ${error instanceof Error ? error.message : String(error)}`); }
    const child = spawn(executable, [], { detached: true, stdio: "ignore", env: process.env });
    let exit: string | undefined;
    child.once("error", (error) => { exit = error.message; });
    child.once("exit", (code, signal) => { exit = `exited with ${code ?? signal}`; });
    child.unref();
    const deadline = Date.now() + 7_500;
    let lastError: unknown;
    while (Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 75));
      try { await this.send("ping", {}); return; } catch (error) { lastError = error; }
    }
    throw new Error(`pi-cron daemon did not start: ${exit ?? (lastError instanceof Error ? lastError.message : String(lastError))}`);
  }
}

function requestTimeout(method: DaemonMethod): number {
  if (method === "ping") return 1_000;
  if (method === "add" || method === "edit") return 65_000;
  if (method === "run_now") {
    const configured = Number(process.env.PI_CRON_RUNNER_TIMEOUT_MS);
    return (Number.isFinite(configured) && configured > 0 ? configured : 15 * 60_000) + 5_000;
  }
  return 10_000;
}
function isUnavailable(error: unknown): boolean {
  const code = (error as NodeJS.ErrnoException)?.code;
  return code === "ENOENT" || code === "ECONNREFUSED" || String(error).includes("ENOENT") || String(error).includes("ECONNREFUSED");
}
