import assert from "node:assert/strict";
import test from "node:test";
import { ensureProtocolFabric } from "@kybernetria/pi-protocol/core";
import extension from "../extension.ts";
import { definition, protocolNodeId } from "../src/protocol-manifest.ts";

const names = ["add", "edit", "delete", "list", "get", "enable", "disable", "run_now"];

test("cron contract is canonical, bounded, and deployment-free", () => {
  assert.equal(protocolNodeId, "pi_cron");
  assert.deepEqual(definition.manifest.provides.map((provide) => provide.name), names);
  assert.equal(JSON.stringify(definition.manifest).includes("execution"), false);
  assert.deepEqual(definition.manifest.provides.find((provide) => provide.name === "run_now")?.effects,
    ["fs.write", "process.spawn", "model.call", "protocol.invoke"]);
});

test("extension installs an owned exact registration and disposes it", async () => {
  let shutdown: (() => Promise<void>) | undefined;
  const commands: string[] = [];
  extension({
    registerCommand(name: string) { commands.push(name); },
    on(name: string, callback: () => Promise<void>) { if (name === "session_shutdown") shutdown = callback; },
  } as never);
  const fabric = ensureProtocolFabric();
  assert.deepEqual(fabric.describeNode(protocolNodeId)?.provides.map((provide) => provide.name), names);
  assert.match(fabric.diagnostics().registrations.find((item) => item.nodeId === protocolNodeId)?.registrationId ?? "", /^registration_/);
  assert.deepEqual(commands, ["cron"]);
  await shutdown?.();
  assert.equal(fabric.describeNode(protocolNodeId), undefined);
});
