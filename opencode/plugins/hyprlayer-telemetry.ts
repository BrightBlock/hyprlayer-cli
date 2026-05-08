import { appendFile, access } from "node:fs/promises";
import { constants as fsConstants } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileP = promisify(execFile);

interface SkillEntry {
  skill: string;
  token: string;
}

const sessions = new Map<string, SkillEntry>();
const LOG_PATH = join(homedir(), ".config", "opencode", "logs", "hyprlayer-telemetry.log");
const DEBUG = process.env.HYPRLAYER_TELEMETRY_DEBUG === "1";

const HYPRLAYER_CANDIDATES = [
  `${homedir()}/.local/bin/hyprlayer`,
  "/opt/homebrew/bin/hyprlayer",
  "/usr/local/bin/hyprlayer",
  "/usr/bin/hyprlayer",
];

function newToken(): string {
  const ms = Date.now();
  const rand = Array.from(crypto.getRandomValues(new Uint8Array(4)))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  return `${ms}-${rand}`;
}

async function diag(line: string): Promise<void> {
  if (!DEBUG) return;
  try {
    await appendFile(LOG_PATH, `${new Date().toISOString()} ${line}\n`);
  } catch {}
}

async function resolveHyprlayer(): Promise<string> {
  for (const path of HYPRLAYER_CANDIDATES) {
    try {
      await access(path, fsConstants.X_OK);
      return path;
    } catch {}
  }
  return "hyprlayer";
}

export const HyprlayerTelemetryPlugin = async (ctx: any) => {
  await diag(`plugin loaded; ctx keys=[${Object.keys(ctx ?? {}).join(",")}]`);

  const hyprlayerPath = await resolveHyprlayer();
  await diag(`resolved hyprlayer=${hyprlayerPath}`);

  async function emitEnd(entry: SkillEntry, outcome: "success" | "failure" = "success"): Promise<void> {
    const args = ["telemetry", "skill-end", "--skill", entry.skill, "--session", entry.token];
    if (outcome === "failure") args.push("--outcome", "failure");
    try {
      await execFileP(hyprlayerPath, args);
      await diag(`skill-end skill=${entry.skill} outcome=${outcome} ok`);
    } catch (err: any) {
      await diag(`skill-end FAILED skill=${entry.skill} err=${err?.message ?? err}`);
    }
  }

  return {
    "command.execute.before": async (input: any) => {
      await diag(`command.execute.before sessionID=${input.sessionID} command=${input.command}`);
      const prior = sessions.get(input.sessionID);
      if (prior) {
        sessions.delete(input.sessionID);
        await emitEnd(prior);
      }
      sessions.set(input.sessionID, { skill: input.command, token: newToken() });
    },
    event: async ({ event }: { event: any }) => {
      if (event?.type === "session.idle") {
        const sessionID = event.properties.sessionID;
        const entry = sessions.get(sessionID);
        if (!entry) return;
        await diag(`session.idle sessionID=${sessionID} skill=${entry.skill}`);
        sessions.delete(sessionID);
        await emitEnd(entry, "success");
      } else if (event?.type === "session.error") {
        const sessionID = event.properties?.sessionID;
        if (!sessionID) return;
        const entry = sessions.get(sessionID);
        if (!entry) return;
        await diag(`session.error sessionID=${sessionID} skill=${entry.skill}`);
        sessions.delete(sessionID);
        await emitEnd(entry, "failure");
      } else if (event?.type === "session.deleted") {
        const sessionID = event.properties?.info?.id;
        if (!sessionID) return;
        const entry = sessions.get(sessionID);
        if (!entry) return;
        await diag(`session.deleted sessionID=${sessionID} skill=${entry.skill}`);
        sessions.delete(sessionID);
        await emitEnd(entry, "success");
      }
    },
  };
};

export default HyprlayerTelemetryPlugin;
