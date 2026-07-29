#!/usr/bin/env node
import { constants } from "node:fs";
import { access, mkdir, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

let daemon = process.env.PI_CRON_DAEMON_BIN || fileURLToPath(new URL("../rust/target/release/pi-cron-daemon", import.meta.url));
if (!isAbsolute(daemon)) daemon = resolve(daemon);
try { await access(daemon, constants.X_OK); }
catch { throw new Error(`Rust daemon binary is not executable at ${daemon}; run npm run build:daemon or set PI_CRON_DAEMON_BIN`); }
const servicePath = join(process.env.XDG_CONFIG_HOME || join(homedir(), ".config"), "systemd", "user", "pi-cron.service");
const quote = (value: string) => `"${value.replaceAll("%", "%%").replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`;
const unit = `[Unit]\nDescription=Pi cron scheduler daemon\nAfter=default.target\n\n[Service]\nType=simple\nExecStart=${quote(daemon)}\nRestart=on-failure\nRestartSec=2\nUMask=0077\n\n[Install]\nWantedBy=default.target\n`;
await mkdir(dirname(servicePath), { recursive: true });
await writeFile(servicePath, unit, { mode: 0o600 });
for (const args of [["--user", "daemon-reload"], ["--user", "enable", "--now", "pi-cron.service"]]) {
  const result = spawnSync("systemctl", args, { stdio: "inherit" });
  if (result.status !== 0) throw new Error(`systemctl ${args.join(" ")} failed`);
}
console.log(`Installed ${servicePath} using ${daemon}`);
