export interface CronJob {
  id: string;
  name: string;
  enabled: boolean;
  schedule: string;
  timezone: string;
  action: ChatAction | ProtocolAction;
  createdAt: string;
  updatedAt: string;
  nextAt: string | null;
}

export interface ChatAction {
  type: "chat";
  prompt: string;
  cwd: string;
}

export interface ProtocolAction {
  type: "protocol";
  target: string;
  input: unknown;
}

export interface JobReplacement {
  id: string;
  name: string;
  enabled: boolean;
  schedule: string;
  timezone: string;
  action: ChatAction | ProtocolAction;
}

export type OccurrenceStatus = "claimed" | "completed" | "skipped";

export interface Occurrence {
  id: number;
  jobId: string;
  scheduledAt: string;
  claimedAt: string;
  status: OccurrenceStatus;
  finishedAt: string | null;
  result: string | null;
  error: string | null;
}

export interface ExecutionResult {
  status: "completed" | "skipped";
  result?: string;
  error?: string;
}

export type DaemonMethod = "add" | "edit" | "delete" | "list" | "get" | "enable" | "disable" | "run_now" | "ping";

export interface DaemonRequest {
  id: string;
  method: DaemonMethod;
  params: unknown;
}

export type DaemonResponse =
  | { id: string; ok: true; result: unknown }
  | { id: string; ok: false; error: { code: string; message: string } };
