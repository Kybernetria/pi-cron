import {
  createAgentSession,
  SessionManager,
  SettingsManager,
} from "@earendil-works/pi-coding-agent";
import type { CronJob, ExecutionResult } from "../src/types.ts";
import { bound } from "./protocol.ts";

export async function executeChat(job: CronJob): Promise<ExecutionResult> {
  if (job.action.type !== "chat") throw new Error("expected chat action");
  const settings = SettingsManager.create(job.action.cwd, undefined, { projectTrusted: false });
  settings.applyOverrides({ retry: { enabled: false } });
  const { session } = await createAgentSession({
    cwd: job.action.cwd,
    sessionManager: SessionManager.inMemory(job.action.cwd),
    settingsManager: settings,
  });
  try {
    await session.prompt(job.action.prompt, { source: "extension" });
    const text = lastAssistantText(session.messages);
    return { status: "completed", result: bound(text || "Pi chat completed without textual output.") };
  } finally {
    session.dispose();
  }
}

function lastAssistantText(messages: readonly unknown[]): string {
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index] as { role?: string; content?: unknown };
    if (message.role !== "assistant" || !Array.isArray(message.content)) continue;
    return message.content.filter((part): part is { type: "text"; text: string } =>
      Boolean(part && typeof part === "object" && (part as { type?: string }).type === "text"))
      .map((part) => part.text).join("").trim();
  }
  return "";
}
