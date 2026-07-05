#!/bin/sh
# zellij-tab-sidebar description_command: summarize the focused pane's visible
# buffer (provided by the plugin in $ZELLIJ_SIDEBAR_PANE_CONTENT) into a short
# one-liner via a cheap one-shot `pi -p` call.
#
# Usage (layout):
#   plugin location="..." {
#       description_command "/path/to/summarize-pane.sh"
#       interval "30"    # LLM call per tab per interval — keep this high!
#   }
#
# Caches per tab: only re-summarizes when the pane content changed.

[ -n "$ZELLIJ_SIDEBAR_PANE_CONTENT" ] || exit 0

CACHE_DIR="${TMPDIR:-/tmp}/zellij-sidebar-summarize"
mkdir -p "$CACHE_DIR"
CACHE="$CACHE_DIR/tab-${ZELLIJ_TAB_POSITION:-0}"

# Hash the content; if unchanged, replay the cached summary.
HASH=$(printf '%s' "$ZELLIJ_SIDEBAR_PANE_CONTENT" | cksum | cut -d' ' -f1)
if [ -f "$CACHE" ]; then
    read -r cached_hash cached_summary < "$CACHE"
    if [ "$cached_hash" = "$HASH" ]; then
        printf '%s\n' "$cached_summary"
        exit 0
    fi
fi

SUMMARY=$(printf '%s' "$ZELLIJ_SIDEBAR_PANE_CONTENT" | pi -p \
    --model anthropic/claude-haiku-4-5 \
    --no-extensions --no-skills --no-tools --no-session \
    "Summarize what is currently happening in this terminal, in Japanese, in
about 10 to 20 characters. Output only the summary, no punctuation, no
quotes, no explanations. If you cannot tell, output exactly: 不明.
Terminal content follows:" 2>/dev/null | tr -s ' \n' ' ' | sed 's/^ //;s/ $//')

# Guard: refusals/explanations are long; 不明 is the explicit sentinel.
case "$SUMMARY" in
    "" | "不明") exit 0 ;;
esac
[ "${#SUMMARY}" -le 90 ] || exit 0  # ~30 JP chars in UTF-8 bytes

printf '%s %s\n' "$HASH" "$SUMMARY" > "$CACHE"
printf '%s\n' "$SUMMARY"
