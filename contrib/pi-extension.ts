/**
 * zellij-tab-sidebar integration for pi (https://github.com/badlogic/pi-mono).
 *
 * Mirrors the agent's state into the sidebar:
 *   description: a short LLM summary of what the session is currently working
 *                on, refreshed on every new prompt from the recent prompt
 *                history (follows the conversation). Left unset if the summary
 *                fails; overlapping refreshes are coalesced.
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

// Summarize the recent prompt history into a few words via a one-shot, ephemeral
// `pi -p` call. Async + fire-and-forget: never blocks the agent. `cb` is always
// invoked exactly once (with "" on failure/timeout) so callers can clear their
// in-flight guard. `--no-extensions` avoids re-loading this extension.
//
// NOTE: must use spawn + stdin.end(), NOT execFile. `pi -p` reads stdin, so an
// open stdin pipe (execFile's default) makes it hang until timeout. Closing
// stdin sends EOF so it returns in ~2s.
function summarize(text: string, cb: (summary: string) => void): void {
  const instruction =
    "Summarize what this coding session is currently working on in 3 to 5 " +
    "words. Output only the summary, lowercase, no punctuation, no quotes. " +
    "Base it on the user's requests below (later ones are more recent):\n\n";
  const child = spawn(
    "pi",
    [
      "-p",
      // Cheap + fast for a few-word summary. Provider must be explicit: the
      // CLI's default provider is google, so a bare "haiku" pattern misresolves.
      "--model",
      "anthropic/claude-haiku-4-5",
      "--no-extensions",
      "--no-skills",
      "--no-tools",
      "--no-session",
      instruction + text,
    ],
    { timeout: 30_000 },
  );
  let out = "";
  let done = false;
  const finish = (s: string) => {
    if (!done) {
      done = true;
      cb(s);
    }
  };
  child.stdout?.on("data", (d) => {
    if (out.length < 4096) out += d;
  });
  child.on("error", () => finish("")); // e.g. pi not on PATH
  child.on("close", () => finish(out.replace(/\s+/g, " ").trim().slice(0, 60)));
  child.stdin?.end(); // send EOF so `pi -p` doesn't wait on stdin
}

export default function (pi: ExtensionAPI) {
  if (!IN_ZELLIJ) return;

  // Recent user prompts drive the description; re-summarized as the session goes.
  let prompts: string[] = [];
  let inFlight = false; // a summarize child is running
  let dirty = false; // a newer prompt arrived while summarizing

  const refresh = (): void => {
    if (inFlight || prompts.length === 0) {
      if (inFlight) dirty = true;
      return;
    }
    inFlight = true;
    // Last few prompts, most-recent-last, bounded so the input stays small.
    const text = prompts.slice(-8).join("\n").slice(-2000);
    summarize(text, (summary) => {
      if (summary) pipe("tab_desc", summary);
      inFlight = false;
      if (dirty) {
        dirty = false;
        refresh(); // coalesce: summarize once more for the prompts we skipped
      }
    });
  };

  // Fresh session -> reset the prompt history.
  pi.on("session_start", async () => {
    prompts = [];
    inFlight = false;
    dirty = false;
  });

  pi.on("before_agent_start", async (event) => {
    const raw = (event.prompt ?? "").replace(/\s+/g, " ").trim();
    if (raw) {
      prompts.push(raw);
      refresh(); // update the description to follow the conversation
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
