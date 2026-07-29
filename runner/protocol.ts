import {
  createAgentSession,
  SessionManager,
  SettingsManager,
  type AgentSession,
} from "@earendil-works/pi-coding-agent";
import {
  ensureProtocolFabric,
  type ProtocolFabric,
  type ProtocolPrincipal,
} from "@kybernetria/pi-protocol/core";
import type { CronJob, ExecutionResult, ProtocolAction } from "../src/types.ts";
import { OUTPUT_LIMIT } from "./types.ts";

export interface ProtocolRuntime {
  session: AgentSession;
  fabric: ProtocolFabric;
  principal: ProtocolPrincipal;
}

export async function bootstrapProtocol(): Promise<ProtocolRuntime> {
  const cwd = process.cwd();
  const settings = SettingsManager.create(cwd, undefined, { projectTrusted: false });
  settings.applyOverrides({ retry: { enabled: false } });
  const { session } = await createAgentSession({ cwd, sessionManager: SessionManager.inMemory(cwd), settingsManager: settings });
  const fabric = ensureProtocolFabric();
  return {
    session,
    fabric,
    principal: fabric.mintPrincipal("pi_cron.runner", "system"),
  };
}

/**
 * Validation is intentionally discovery-only here. The canonical fabric is the
 * sole schema validator and performs bounded input admission immediately before
 * scheduled execution.
 */
export function validateProtocol(runtime: ProtocolRuntime, action: ProtocolAction): void {
  const [nodeId, provide] = splitTarget(action.target);
  if (!runtime.fabric.describeProvide(nodeId, provide)) {
    throw new Error(`protocol target does not exist: ${action.target}`);
  }
}

export async function executeProtocol(runtime: ProtocolRuntime, job: CronJob): Promise<ExecutionResult> {
  if (job.action.type !== "protocol") throw new Error("expected protocol action");
  const target = job.action.target;
  const result = await runtime.fabric.invokeAs(runtime.principal, target, job.action.input, {
    grant: { targets: [target], maxDepth: 8, maxInvocations: 64 },
  });
  const receipt = {
    invocationId: result.receipt.invocationId,
    traceId: result.receipt.traceId,
    spanId: result.receipt.spanId,
    state: result.receipt.state,
    target: result.receipt.target,
  };
  if (!result.ok) {
    return { status: "completed", error: bound(`${result.error.code}: ${result.error.message}\nreceipt=${JSON.stringify(receipt)}`) };
  }
  return { status: "completed", result: bound(`${serialize(result.output)}\nreceipt=${JSON.stringify(receipt)}`) };
}

export function splitTarget(target: string): [string, string] {
  const parts = target.split(".");
  if (parts.length !== 2 || !parts[0] || !parts[1]) throw new Error("target must be one exact node.provide");
  return [parts[0], parts[1]];
}

function serialize(value: unknown): string {
  if (typeof value === "string") return value;
  try { return JSON.stringify(value); } catch { return String(value); }
}
export function bound(value: string): string {
  return value.length <= OUTPUT_LIMIT ? value : `${value.slice(0, OUTPUT_LIMIT)}\n[truncated]`;
}
