import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, realpath, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";
import { createConnection } from "node:net";
import test from "node:test";
import { DatabaseSync } from "node:sqlite";
import { CronClient } from "../src/client.ts";

const binary = new URL("../rust/target/debug/pi-cron-daemon", import.meta.url).pathname;

test("TS client uses Rust daemon with an existing v1 database", async () => {
  const dir = await mkdtemp(join(tmpdir(), "pi-cron-rust-"));
  const db = join(dir, "state", "jobs.sqlite"); const runtime = join(dir, "runtime"); const socket = join(runtime, "daemon.sock");
  await mkdir(join(dir, "state"));
  const existing = new DatabaseSync(db);
  existing.exec(`CREATE TABLE schema_version (version INTEGER NOT NULL); INSERT INTO schema_version VALUES(1);
    CREATE TABLE jobs (id TEXT PRIMARY KEY,name TEXT NOT NULL,enabled INTEGER NOT NULL,schedule TEXT NOT NULL,timezone TEXT NOT NULL,action_json TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL,next_at TEXT);`);
  const stamp = new Date().toISOString();
  existing.prepare("INSERT INTO jobs VALUES(?,?,?,?,?,?,?,?,?)").run("existing", "Existing", 0, "0 * * * *", "UTC", JSON.stringify({ type: "protocol", target: "fake.call", input: {} }), stamp, stamp, null);
  existing.close();
  const fake = join(dir, "fake-runner"); const marker = join(dir, "runner-started");
  await writeFile(fake, `#!/usr/bin/env node\nlet s='';process.stdin.on('data',c=>s+=c).on('end',()=>{let r=JSON.parse(s);if(r.operation==='validate_protocol')return process.stdout.write(JSON.stringify({version:1,ok:true,result:{valid:true}})+'\\n');let mode=r.job.action.input.mode;if(mode==='crash')process.exit(9);if(mode==='hang'){require('fs').writeFileSync(process.env.CRON_TEST_MARKER,'');return setTimeout(()=>{},10000)}let result={status:mode==='status'?'invalid':'completed',result:mode==='oversize'?'x'.repeat(140000):mode==='cwd'?process.cwd():mode==='env'?process.env.CRON_TEST_ENV:'fake-ok'};process.stdout.write(JSON.stringify({version:1,ok:true,result})+'\\n')})\n`);
  await chmod(fake, 0o700);
  const env = { ...process.env, PI_CRON_DB: db, PI_CRON_RUNTIME_DIR: runtime, PI_CRON_RUNNER_BIN: fake, PI_CRON_RUNNER_TIMEOUT_MS: "100", CRON_TEST_ENV: "preserved", CRON_TEST_MARKER: marker };
  const daemon = spawn(binary, [], { env, stdio: ["ignore", "ignore", "pipe"] });
  let diagnostics = ""; daemon.stderr.setEncoding("utf8"); daemon.stderr.on("data", (chunk) => diagnostics += chunk);
  const client = new CronClient(socket, false);
  let stopped = false;
  try {
    await eventually(() => client.request("ping"));
    assert.equal((await client.request<Array<{ id: string }>>("list"))[0]?.id, "existing");
    const occurrence = await client.request<{ status: string; result: string }>("run_now", { id: "existing" });
    assert.equal(occurrence.status, "completed"); assert.equal(occurrence.result, "fake-ok");
    for (const [mode, result] of [["cwd", await realpath(process.env.HOME!)], ["env", "preserved"]]) {
      const added = await client.request<{ id: string }>("add", { job: { name: mode, enabled: false, schedule: "0 * * * *", timezone: "UTC", action: { type: "protocol", target: "fake.call", input: { mode } } } });
      assert.equal((await client.request<{ result: string }>("run_now", { id: added.id })).result, result);
    }
    await assert.rejects(client.request("add", { job: { name: "recursive", enabled: false, schedule: "0 * * * *", timezone: "UTC", action: { type: "protocol", target: "pi_cron.list", input: {} } } }), /cannot schedule its own/);
    await Promise.all(Array.from({ length: 20 }, () => client.request("ping")));
    const malformed = await rawRequest(socket, "{bad json\n");
    assert.equal(malformed.ok, false); assert.equal(malformed.error.code, "INVALID_REQUEST");
    for (const mode of ["crash", "hang", "oversize", "status"]) {
      const added = await client.request<{ id: string }>("add", { job: { name: mode, enabled: false, schedule: "0 * * * *", timezone: "UTC", action: { type: "protocol", target: "fake.call", input: { mode } } } });
      const failed = await client.request<{ error: string }>("run_now", { id: added.id });
      assert.match(failed.error, mode === "hang" ? /timed out/ : mode === "oversize" ? /too large/ : mode === "status" ? /invalid runner execution response/ : /invalid JSON|exited/);
    }
    assert.equal((await stat(runtime)).mode & 0o777, 0o700);
    assert.equal((await stat(socket)).mode & 0o777, 0o600);
    assert.equal((await stat(db)).mode & 0o777, 0o600);

    const second = spawn(binary, [], { env, stdio: ["ignore", "ignore", "pipe"] });
    const secondError = await collectExit(second);
    assert.notEqual(secondError.code, 0); assert.match(secondError.stderr, /already running/);

    await rm(marker, { force: true });
    const shutdownJob = await client.request<{ id: string }>("add", { job: { name: "shutdown", enabled: false, schedule: "0 * * * *", timezone: "UTC", action: { type: "protocol", target: "fake.call", input: { mode: "hang" } } } });
    const pending = client.request<{ status: string; error: string }>("run_now", { id: shutdownJob.id });
    await eventually(() => stat(marker));
    const exit = collectExit(daemon);
    daemon.kill("SIGTERM");
    const [interrupted, exited] = await Promise.all([pending, exit]);
    stopped = true;
    assert.match(interrupted.error, /cancelled during daemon shutdown/);
    assert.equal(exited.code, 0, diagnostics);
  } finally {
    if (!stopped) { const exit = collectExit(daemon); daemon.kill("SIGTERM"); assert.equal((await exit).code, 0, diagnostics); }
    await assert.rejects(stat(socket));
    await rm(dir, { recursive: true, force: true });
  }
});

async function eventually(fn: () => Promise<unknown>): Promise<void> {
  const until = Date.now() + 5_000; let error: unknown;
  while (Date.now() < until) { try { await fn(); return; } catch (caught) { error = caught; await new Promise(resolve => setTimeout(resolve, 30)); } }
  throw error;
}
async function rawRequest(path: string, request: string): Promise<any> {
  return await new Promise((resolve, reject) => { const socket = createConnection(path); let data = ""; socket.setEncoding("utf8"); socket.once("connect", () => socket.end(request)); socket.on("data", chunk => data += chunk); socket.once("error", reject); socket.once("end", () => resolve(JSON.parse(data))); });
}
function collectExit(child: ReturnType<typeof spawn>): Promise<{ code: number | null; stderr: string }> {
  let stderr = ""; child.stderr?.setEncoding("utf8"); child.stderr?.on("data", chunk => stderr += chunk);
  return new Promise((resolve, reject) => { child.once("error", reject); child.once("exit", code => resolve({ code, stderr })); });
}
