import { fileURLToPath } from "node:url";
import type { ExtensionAPI, ExtensionCommandContext } from "@earendil-works/pi-coding-agent";
import { ensureProtocolFabric, type ProtocolHandler } from "@kybernetria/pi-protocol/core";
import { CronClient } from "./src/client.ts";
import { definition } from "./src/protocol-manifest.ts";
import type { CronJob, JobReplacement, Occurrence } from "./src/types.ts";

export default function piCronExtension(pi: ExtensionAPI): void {
  const client = new CronClient();
  const fabric = ensureProtocolFabric();
  const handlers = Object.fromEntries(definition.manifest.provides.map((provide) => [
    provide.name,
    (async (input: unknown) => client.request(provide.name as Parameters<CronClient["request"]>[0], input)) as ProtocolHandler,
  ]));
  const registration = fabric.install(definition, { handlers }, {
    packageId: "pi-cron",
    packageVersion: "0.1.0",
    sourcePath: fileURLToPath(new URL(".", import.meta.url)),
  });
  pi.on("session_shutdown", async () => { await registration.dispose(); });

  pi.registerCommand("cron", {
    description: "Cron manager. Run /cron with no arguments for the guided UI.",
    getArgumentCompletions: (prefix) => ["list", "add", "edit ", "delete ", "run ", "enable ", "disable ", "help"]
      .filter((value) => value.startsWith(prefix)).map((value) => ({ value, label: value.trim() })),
    handler: async (args, ctx) => {
      try {
        await handleCommand(pi, client, (args ?? "").trim(), ctx);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        ctx.ui.notify(message, "error");
        show(pi, `**Pi cron error**\n\n${message}`);
      }
    },
  });
}

async function handleCommand(pi: ExtensionAPI, client: CronClient, args: string, ctx: ExtensionCommandContext): Promise<void> {
  if (!args) {
    if (ctx.hasUI) await runInteractiveCron(pi, client, ctx);
    else await showList(pi, client);
    return;
  }

  const [rawCommand, ...rest] = args.split(/\s+/);
  const command = rawCommand.toLowerCase();
  const suppliedId = rest[0];
  switch (command) {
    case "list": case "ls": case "status": await showList(pi, client); return;
    case "help": case "--help": case "-h": showHelp(pi); return;
    case "add": case "new": case "create": requireInteractive(ctx); await createJobFlow(pi, client, ctx); return;
    case "edit": {
      requireInteractive(ctx);
      const job = await resolveJob(client, ctx, suppliedId, "Choose a job to edit");
      if (job) await editJobFlow(pi, client, ctx, job);
      return;
    }
    case "delete": case "remove": {
      requireInteractive(ctx);
      const job = await resolveJob(client, ctx, suppliedId, "Choose a job to delete");
      if (job) await deleteJobFlow(client, ctx, job);
      return;
    }
    case "run": case "run-now": {
      requireInteractive(ctx);
      const job = await resolveJob(client, ctx, suppliedId, "Choose a job to run now");
      if (job) await runJobFlow(client, ctx, job);
      return;
    }
    case "enable": case "disable": {
      const job = suppliedId ? undefined : (ctx.hasUI ? await chooseJob(client, ctx, `Choose a job to ${command}`) : undefined);
      const id = suppliedId ?? job?.id;
      if (!id) { showHelp(pi); return; }
      const updated = await client.request<CronJob>(command, { id });
      ctx.ui.notify(`${updated.enabled ? "Enabled" : "Disabled"} ${updated.name}`, "info");
      return;
    }
    default:
      // Unknown input should never strand the user at a terse usage error.
      showHelp(pi);
  }
}

async function runInteractiveCron(pi: ExtensionAPI, client: CronClient, ctx: ExtensionCommandContext): Promise<void> {
  while (true) {
    const jobs = await client.request<CronJob[]>("list");
    const enabled = jobs.filter((job) => job.enabled).length;
    const choice = await ctx.ui.select(
      `Cron jobs (${enabled} enabled, ${jobs.length - enabled} disabled): what would you like to do?`,
      ["➕ Create a cron job", "📋 Browse and manage jobs", "📊 Show a quick summary", "❔ Show help", "Done"],
    );
    if (!choice || choice === "Done") return;
    try {
      if (choice.startsWith("➕")) await createJobFlow(pi, client, ctx);
      else if (choice.startsWith("📋")) await browseJobs(pi, client, ctx);
      else if (choice.startsWith("📊")) await showList(pi, client);
      else if (choice.startsWith("❔")) showHelp(pi);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      ctx.ui.notify(message, "error");
    }
  }
}

async function browseJobs(pi: ExtensionAPI, client: CronClient, ctx: ExtensionCommandContext): Promise<void> {
  while (true) {
    const job = await chooseJob(client, ctx, "Choose a cron job");
    if (!job) return;
    const leave = await jobActionFlow(pi, client, ctx, job.id);
    if (leave) return;
  }
}

async function jobActionFlow(pi: ExtensionAPI, client: CronClient, ctx: ExtensionCommandContext, id: string): Promise<boolean> {
  while (true) {
    const current = await client.request<{ job: CronJob; occurrences: Occurrence[] }>("get", { id });
    const job = current.job;
    const choice = await ctx.ui.select(formatJobLine(job), [
      "👀 Show details", "▶️ Run now", "✏️ Edit", job.enabled ? "⏸️ Disable" : "✅ Enable", "🗑️ Delete", "← Back",
    ]);
    if (!choice || choice === "← Back") return false;
    if (choice.startsWith("👀")) { show(pi, `**${job.name}**\n\n${formatJobDetail(job, current.occurrences[0])}`); continue; }
    if (choice.startsWith("▶️")) { await runJobFlow(client, ctx, job); continue; }
    if (choice.startsWith("✏️")) { await editJobFlow(pi, client, ctx, job); continue; }
    if (choice.startsWith("⏸️") || choice.startsWith("✅")) {
      const updated = await client.request<CronJob>(job.enabled ? "disable" : "enable", { id: job.id });
      ctx.ui.notify(`${updated.enabled ? "Enabled" : "Disabled"} ${updated.name}`, "info");
      continue;
    }
    if (choice.startsWith("🗑️")) return await deleteJobFlow(client, ctx, job);
  }
}

async function createJobFlow(pi: ExtensionAPI, client: CronClient, ctx: ExtensionCommandContext): Promise<void> {
  const name = (await ctx.ui.input("New cron job name:", "Morning project review"))?.trim();
  if (!name) return;
  const schedule = (await ctx.ui.input("Five-field cron schedule:", "0 9 * * 1-5"))?.trim();
  if (!schedule) return;
  // Use the host's configured timezone by default. Timezone remains explicit in
  // storage so schedules stay deterministic if the device setting changes later.
  const timezone = deviceTimezone();
  const action = await actionFlow(ctx);
  if (!action) return;
  const draft = { name, enabled: true, schedule, timezone, action };
  if (!await ctx.ui.confirm("Create cron job?", formatDraft(draft))) return;
  const created = await client.request<CronJob>("add", { job: draft });
  ctx.ui.notify(`Created ${created.name}`, "info");
  show(pi, `**Created cron job**\n\n${formatJobDetail(created)}`);
}

async function editJobFlow(pi: ExtensionAPI, client: CronClient, ctx: ExtensionCommandContext, job: CronJob): Promise<void> {
  const name = (await ctx.ui.editor("Job name:", job.name))?.trim();
  if (!name) return;
  const schedule = (await ctx.ui.editor("Five-field cron schedule:", job.schedule))?.trim();
  if (!schedule) return;
  const timezone = (await ctx.ui.editor("IANA timezone:", job.timezone))?.trim();
  if (!timezone) return;
  const enabledChoice = await ctx.ui.select("Job state:", [job.enabled ? "Enabled (current)" : "Disabled (current)", job.enabled ? "Disabled" : "Enabled"]);
  if (!enabledChoice) return;
  const enabled = enabledChoice.startsWith("Enabled");
  const actionChoice = await ctx.ui.select("Action:", ["Keep current action", "💬 Configure chat action", "🔌 Configure protocol action"]);
  if (!actionChoice) return;
  const action = actionChoice === "Keep current action" ? job.action : await actionFlow(ctx, actionChoice.startsWith("💬") ? "chat" : "protocol", job);
  if (!action) return;
  const replacement: JobReplacement = { id: job.id, name, enabled, schedule, timezone, action };
  if (!await ctx.ui.confirm("Save complete replacement?", formatDraft(replacement))) return;
  const updated = await client.request<CronJob>("edit", { job: replacement });
  ctx.ui.notify(`Updated ${updated.name}`, "info");
  show(pi, `**Updated cron job**\n\n${formatJobDetail(updated)}`);
}

async function actionFlow(
  ctx: ExtensionCommandContext,
  forcedType?: "chat" | "protocol",
  existing?: CronJob,
): Promise<JobReplacement["action"] | undefined> {
  const selected = forcedType ?? await ctx.ui.select("What should this job do?", ["💬 Run an isolated Pi chat", "🔌 Invoke an exact protocol target"]);
  if (!selected) return undefined;
  const type = selected === "chat" || selected.startsWith("💬") ? "chat" : "protocol";
  if (type === "chat") {
    const old = existing?.action.type === "chat" ? existing.action : undefined;
    const prompt = (await ctx.ui.editor("Chat prompt:", old?.prompt ?? ""))?.trim();
    if (!prompt) return undefined;
    const cwd = (await ctx.ui.editor("Absolute working directory:", old?.cwd ?? ctx.cwd))?.trim();
    if (!cwd) return undefined;
    return { type: "chat", prompt, cwd };
  }
  const old = existing?.action.type === "protocol" ? existing.action : undefined;
  const target = (await ctx.ui.editor("Exact protocol target (node.provide):", old?.target ?? ""))?.trim();
  if (!target) return undefined;
  const inputText = await ctx.ui.editor("Protocol input (JSON):", JSON.stringify(old?.input ?? {}, null, 2));
  if (inputText === undefined) return undefined;
  return { type: "protocol", target, input: parseJson(inputText) };
}

async function runJobFlow(client: CronClient, ctx: ExtensionCommandContext, job: CronJob): Promise<void> {
  if (!await ctx.ui.confirm("Run cron job now?", `${job.name}\n${describeAction(job)}`)) return;
  ctx.ui.notify(`Running ${job.name}…`, "info");
  const occurrence = await client.request<{ status: string; error?: string }>("run_now", { id: job.id });
  ctx.ui.notify(occurrence.error ? `Completed with error: ${occurrence.error}` : `Run ${occurrence.status}`, occurrence.error ? "warning" : "info");
}

async function deleteJobFlow(client: CronClient, ctx: ExtensionCommandContext, job: CronJob): Promise<boolean> {
  if (!await ctx.ui.confirm("Delete cron job?", `${job.name} (${job.id})\n\nThis cannot be undone.`)) return false;
  await client.request("delete", { id: job.id });
  ctx.ui.notify(`Deleted ${job.name}`, "info");
  return true;
}

async function resolveJob(client: CronClient, ctx: ExtensionCommandContext, id: string | undefined, title: string): Promise<CronJob | undefined> {
  if (id) return (await client.request<{ job: CronJob }>("get", { id })).job;
  return chooseJob(client, ctx, title);
}

async function chooseJob(client: CronClient, ctx: ExtensionCommandContext, title: string): Promise<CronJob | undefined> {
  const jobs = await client.request<CronJob[]>("list");
  if (!jobs.length) { ctx.ui.notify("No cron jobs yet. Choose Create from /cron to add one.", "info"); return undefined; }
  const labels = jobs.map((job) => formatJobLine(job));
  const selected = await ctx.ui.select(title, [...labels, "← Back"]);
  if (!selected || selected === "← Back") return undefined;
  return jobs[labels.indexOf(selected)];
}

async function showList(pi: ExtensionAPI, client: CronClient): Promise<void> {
  const jobs = await client.request<CronJob[]>("list");
  const text = jobs.length ? jobs.map((job) => `${job.enabled ? "●" : "○"} **${job.name}** \`${job.id}\` — \`${job.schedule}\` (${job.timezone})${job.nextAt ? ` — next ${job.nextAt}` : ""}`).join("\n") : "No cron jobs. Run `/cron` in interactive mode to create one.";
  show(pi, `**Pi cron jobs**\n\n${text}`);
}

function showHelp(pi: ExtensionAPI): void {
  show(pi, ["**⏰ Pi cron**", "", "Run `/cron` with no arguments for the guided manager.", `New jobs automatically use this device's timezone: **${deviceTimezone()}**.`, "", "Optional direct commands:", "- `/cron list`", "- `/cron add`", "- `/cron edit [id]`", "- `/cron delete [id]`", "- `/cron run [id]`", "- `/cron enable <id>` / `/cron disable <id>`"].join("\n"));
}
function formatJobLine(job: CronJob): string { return `${job.enabled ? "●" : "○"} ${job.name} — ${job.schedule} (${job.timezone}) — ${job.id}`; }
function formatJobDetail(job: CronJob, last?: Occurrence): string {
  return [`- ID: \`${job.id}\``, `- State: ${job.enabled ? "enabled" : "disabled"}`, `- Schedule: \`${job.schedule}\``, `- Timezone: \`${job.timezone}\``, `- Next: ${job.nextAt ?? "not scheduled"}`, `- Action: ${describeAction(job)}`,
    ...(last ? [`- Last run: ${last.status} at ${last.scheduledAt}${last.error ? ` — ${last.error}` : ""}`] : []),
  ].join("\n");
}
function describeAction(job: Pick<CronJob, "action">): string { return job.action.type === "chat" ? `chat in ${job.action.cwd}: ${job.action.prompt}` : `protocol ${job.action.target} with ${JSON.stringify(job.action.input)}`; }
function formatDraft(job: Pick<JobReplacement, "name" | "enabled" | "schedule" | "timezone" | "action">): string { return `${job.name}\n${job.enabled ? "Enabled" : "Disabled"} • ${job.schedule} • ${job.timezone}\n${describeAction(job)}`; }
function deviceTimezone(): string { return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC"; }
function requireInteractive(ctx: ExtensionCommandContext): void { if (!ctx.hasUI) throw new Error("This operation needs interactive Pi. Run /cron with UI enabled or use pi_cron protocol provides."); }
function parseJson(value: string): unknown { try { return JSON.parse(value); } catch (error) { throw new Error(`Invalid protocol input JSON: ${error instanceof Error ? error.message : String(error)}`); } }
function show(pi: ExtensionAPI, content: string): void { pi.sendMessage({ customType: "pi-cron.command", content, display: true }); }
