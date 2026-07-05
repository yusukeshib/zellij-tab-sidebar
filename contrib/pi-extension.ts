/**
 * zellij-tab-sidebar integration for pi (https://github.com/badlogic/pi-mono).
 *
 * Mechanically mirrors the agent's state into the sidebar (no LLM involved):
 *   description: the task the agent is working on (taken from the user prompt)
 *   status:      running (while the agent works) | idle (waiting for input)
 *
 * Install: copy or symlink into ~/.pi/agent/extensions/
 *   ln -s /path/to/contrib/pi-extension.ts ~/.pi/agent/extensions/zellij-sidebar.ts
 *
 * No-op when pi is not running inside a zellij pane.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execFile } from "node:child_process";

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

export default function (pi: ExtensionAPI) {
  if (!IN_ZELLIJ) return;

  // The user's prompt IS the task the agent is now working on.
  pi.on("before_agent_start", async (event) => {
    const task = (event.prompt ?? "").replace(/\s+/g, " ").trim().slice(0, 48);
    if (task) pipe("tab_desc", task);
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
