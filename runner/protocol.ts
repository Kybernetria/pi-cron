import { randomUUID } from "node:crypto";
import { Ajv } from "ajv";
import {
  createAgentSession,
  SessionManager,
  SettingsManager,
  type AgentSession,
} from "@earendil-works/pi-coding-agent";
import {
  ensureProtocolFabric,
  type InvocationProvenanceEvent,
  type ProtocolFabric,
} from "@kybernetria/pi-protocol";
import { protocolNamespace } from "../src/protocol-manifest.ts";
import type { CronJob, ExecutionResult, ProtocolAction } from "../src/types.ts";
import { OUTPUT_LIMIT } from "./types.ts";

export interface ProtocolRuntime {
  session: AgentSession;
  fabric: ProtocolFabric;
}

export async function bootstrapProtocol(): Promise<ProtocolRuntime> {
  const cwd = process.cwd();
  const settings = SettingsManager.create(cwd, undefined, { projectTrusted: false });
  settings.applyOverrides({ retry: { enabled: false } });
  const { session } = await createAgentSession({ cwd, sessionManager: SessionManager.inMemory(cwd), settingsManager: settings });
  const fabric = ensureProtocolFabric();
  fabric.unregister(protocolNamespace.nodeId);
  return { session, fabric };
}

export function validateProtocol(runtime: ProtocolRuntime, action: ProtocolAction): void {
  const [nodeId, provide] = splitTarget(action.target);
  const spec = runtime.fabric.describeProvide(nodeId, provide);
  if (!spec) throw new Error(`protocol target does not exist: ${action.target}`);
  const ajv = new Ajv({ allErrors: true, strict: false });
  const valid = ajv.compile(spec.inputSchema as object);
  if (!valid(action.input)) {
    throw new Error(`protocol input does not match ${action.target}: ${ajv.errorsText(valid.errors, { separator: "; " })}`);
  }
}

export async function executeProtocol(runtime: ProtocolRuntime, job: CronJob): Promise<ExecutionResult> {
  if (job.action.type !== "protocol") throw new Error("expected protocol action");
  const [nodeId, provide] = splitTarget(job.action.target);
  const traceId = `cron-${job.id}-${randomUUID()}`;
  const events: InvocationProvenanceEvent[] = [];
  const unsubscribe = runtime.fabric.subscribeProvenanceRecorder((event) => {
    if (event.traceId === traceId) events.push(event);
  });
  try {
    const result = await runtime.fabric.invoke({
      nodeId, provide, input: job.action.input, traceId,
      spanId: `cron-${randomUUID()}`,
      callerNodeId: protocolNamespace.nodeId,
      session: { id: `cron-${randomUUID()}`, mode: "ephemeral" },
    });
    const trace = events.map(({ traceId, spanId, parentSpanId, status }) => ({ traceId, spanId, parentSpanId, status }));
    if (!result.ok) return { status: "completed", error: bound(`${result.error.code}: ${result.error.message}\ntrace=${JSON.stringify(trace)}`) };
    return { status: "completed", result: bound(`${serialize(result.output)}\ntrace=${JSON.stringify(trace)}`) };
  } finally {
    unsubscribe();
  }
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
