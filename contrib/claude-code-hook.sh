#!/bin/sh
# zellij-tab-sidebar integration for Claude Code hooks.
#
# Mechanically mirrors the agent's state into the sidebar (no LLM involved):
#   description: the task the agent is working on (taken from the user prompt)
#   status:      running (prompt/tool) | idle (stop) | yellow:waiting (notification)
#
# Usage (in ~/.claude/settings.json hooks):
#   claude-code-hook.sh prompt        <- UserPromptSubmit
#   claude-code-hook.sh post-tool     <- PostToolUse (matcher: "")
#   claude-code-hook.sh stop          <- Stop
#   claude-code-hook.sh notification  <- Notification
#
# Requires: jq, and Claude Code running inside a zellij pane.

# Not inside zellij -> consume stdin and do nothing.
if [ -z "$ZELLIJ" ] || [ -z "$ZELLIJ_PANE_ID" ]; then
    cat >/dev/null
    exit 0
fi

pipe() {
    zellij pipe --name "$1" --args "pane_id=$ZELLIJ_PANE_ID" -- "$2" >/dev/null 2>&1 || true
}

case "$1" in
prompt)
    # The user's prompt IS the task the agent is now working on.
    # Collapse whitespace and keep the first 48 characters (codepoint-safe).
    task=$(jq -r '.prompt // empty | gsub("\\s+"; " ") | .[0:48]')
    [ -n "$task" ] && pipe tab_desc "$task"
    pipe tab_status "running"
    ;;

post-tool)
    # Still working (covers long turns where no other event fires).
    cat >/dev/null
    pipe tab_status "running"
    ;;

stop)
    cat >/dev/null
    pipe tab_status "idle"
    ;;

notification)
    cat >/dev/null
    pipe tab_status "yellow:waiting"
    ;;

*)
    cat >/dev/null
    ;;
esac

exit 0
