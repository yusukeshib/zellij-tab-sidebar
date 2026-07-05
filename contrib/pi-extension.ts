/**
 * zellij-tab-sidebar integration for pi (https://github.com/badlogic/pi-mono).
 *
 * Mirrors the agent's state into the sidebar:
 *   description: a short LLM summary of the FIRST prompt of the session
 *                (set once; raw prompt shown instantly, summary swapped in when
 *                ready; falls back to the raw prompt if summarization fails)
 *   status:      running (while the agent works) | idle (waiting for input)
 *
 * Install: copy or symlink into ~/.pi/agent/extensions/
 *   ln -s /path/to/contrib/pi-extension.ts ~/.pi/agent/extensions/zellij-sidebar.ts
 *
 * No-op when pi is not running inside a zellij pane.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execFile, spawn } from "node:child_process";

const PANE_ID = process.env.ZELLIJ_PANE_ID;
const IN_ZELLIJ = Boolean(process.env.ZELLIJ && PANE_ID);

function pipe(name: string, payload: string): void {
  execFile(
    "zellij",
    ["pipe", "--name", name, "--args", `pane_id=${PANE_ID}`, "--", payload],
    () => {
      /* best-effort: ignore errors */
    },
  );
}

// Summarize the prompt into a few words via a one-shot, ephemeral `pi -p` call.
// Async + fire-and-forget: never blocks the agent, and any failure/timeout just
// leaves the raw-prompt fallback in place. `--no-extensions` avoids re-loading
// this extension (no recursion).
//
// NOTE: must use spawn + stdin.end(), NOT execFile. `pi -p` reads stdin, so an
// open stdin pipe (execFile's default) makes it hang until timeout. Closing
// stdin sends EOF so it returns in ~2s.
function summarize(prompt: string, cb: (summary: string) => void): void {
  const instruction =
    "Summarize this coding task in 3 to 5 words. Output only the summary, " +
    "lowercase, no punctuation, no quotes. Task: ";
  const child = spawn(
    "pi",
    [
      "-p",
      // Cheap + fast for a few-word summary. Provider must be explicit: the
      // CLI's default provider is google, so a bare "haiku" pattern misresolves.
      // If this model isn't available, summarize just yields nothing and the
      // raw-prompt fallback stays in place.
      "--model",
      "anthropic/claude-haiku-4-5",
      "--no-extensions",
      "--no-skills",
      "--no-tools",
      "--no-session",
      instruction + prompt,
    ],
    { timeout: 30_000 },
  );
  let out = "";
  child.stdout?.on("data", (d) => {
    if (out.length < 4096) out += d;
  });
  child.on("error", () => {}); // e.g. pi not on PATH -> keep the fallback
  child.on("close", () => {
    const s = out.replace(/\s+/g, " ").trim().slice(0, 60);
    if (s) cb(s);
  });
  child.stdin?.end(); // send EOF so `pi -p` doesn't wait on stdin
}

export default function (pi: ExtensionAPI) {
  if (!IN_ZELLIJ) return;

  // Description is set once per session (the first prompt = the session's task).
  let descSet = false;

  // Fresh session -> allow the description to be set again from its first prompt.
  pi.on("session_start", async () => {
    descSet = false;
  });

  pi.on("before_agent_start", async (event) => {
    // Only the first prompt of the session becomes the description.
    if (!descSet) {
      const raw = (event.prompt ?? "").replace(/\s+/g, " ").trim();
      if (raw) {
        descSet = true;
        pipe("tab_desc", raw.slice(0, 48)); // instant fallback
        summarize(raw, (summary) => pipe("tab_desc", summary)); // swap in when ready
      }
    }
    pipe("tab_status", "running");
  });

  // Agent finished -> waiting for the next prompt.
  pi.on("agent_end", async () => {
    pipe("tab_status", "idle");
  });

  // Session over -> clear overrides (sidebar falls back to its defaults).
  pi.on("session_shutdown", async () => {
    pipe("tab_status", "");
    pipe("tab_desc", "");
  });
}
