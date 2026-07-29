import type { CronJob, ExecutionResult, ProtocolAction } from "../src/types.ts";

export const RUNNER_PROTOCOL_VERSION = 1;
export const RUNNER_REQUEST_LIMIT = 1_048_576;
export const OUTPUT_LIMIT = 40_000;

export type RunnerRequest =
  | { version: 1; operation: "validate_protocol"; action: ProtocolAction }
  | { version: 1; operation: "execute"; job: CronJob };

export type RunnerResponse =
  | { version: 1; ok: true; result: ExecutionResult | { valid: true } }
  | { version: 1; ok: false; error: { code: string; message: string } };
