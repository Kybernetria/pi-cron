#!/usr/bin/env node
import { executeChat } from "./chat.ts";
import { bootstrapProtocol, executeProtocol, validateProtocol, type ProtocolRuntime } from "./protocol.ts";
import {
  RUNNER_PROTOCOL_VERSION,
  RUNNER_REQUEST_LIMIT,
  type RunnerRequest,
  type RunnerResponse,
} from "./types.ts";

let wrote = false;
try {
  const request = await readRequest();
  let runtime: ProtocolRuntime | undefined;
  try {
    if (request.operation === "validate_protocol") {
      runtime = await bootstrapProtocol();
      validateProtocol(runtime, request.action);
      respond({ version: 1, ok: true, result: { valid: true } });
    } else if (request.job.action.type === "chat") {
      respond({ version: 1, ok: true, result: await executeChat(request.job) });
    } else {
      runtime = await bootstrapProtocol();
      respond({ version: 1, ok: true, result: await executeProtocol(runtime, request.job) });
    }
  } finally {
    runtime?.session.dispose();
  }
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`[pi-cron runner] ${message}`);
  if (!wrote) respond({ version: 1, ok: false, error: { code: "RUNNER_FAILURE", message } });
  process.exitCode = 1;
}

async function readRequest(): Promise<RunnerRequest> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of process.stdin) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    size += buffer.length;
    if (size > RUNNER_REQUEST_LIMIT) throw new Error("runner request too large");
    chunks.push(buffer);
  }
  if (!size) throw new Error("runner request is empty");
  const text = Buffer.concat(chunks).toString("utf8");
  const request = JSON.parse(text) as Partial<RunnerRequest>;
  if (!request || request.version !== RUNNER_PROTOCOL_VERSION) throw new Error("unsupported runner protocol version");
  if (request.operation !== "validate_protocol" && request.operation !== "execute") throw new Error("unknown runner operation");
  if (request.operation === "validate_protocol" && (!request.action || request.action.type !== "protocol")) throw new Error("validate_protocol requires a protocol action");
  if (request.operation === "execute" && (!request.job || typeof request.job !== "object")) throw new Error("execute requires a job");
  return request as RunnerRequest;
}

function respond(response: RunnerResponse): void {
  if (wrote) throw new Error("runner attempted multiple responses");
  wrote = true;
  process.stdout.write(`${JSON.stringify(response)}\n`);
}
