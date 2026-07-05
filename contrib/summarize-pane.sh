#!/bin/sh
# zellij-tab-sidebar description_command: summarize the focused pane's visible
# buffer (provided by the plugin in $ZELLIJ_SIDEBAR_PANE_CONTENT) into a short
# one-liner via a local Ollama model — free, offline, no API cost.
#
# Requires: `ollama serve` running and the model pulled
# (`ollama pull qwen2.5:7b`). Override with $ZELLIJ_SIDEBAR_OLLAMA_MODEL
# (qwen2.5:3b is faster but noticeably less accurate for this task).
#
# Usage (layout):
#   plugin location="..." {
#       description_command "/path/to/summarize-pane.sh"
#       interval "120"   # summarize per tab per interval
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

MODEL="${ZELLIJ_SIDEBAR_OLLAMA_MODEL:-qwen2.5:7b}"
HOST="${OLLAMA_HOST:-http://localhost:11434}"

# A concrete few-shot prompt keeps small models from echoing the input or
# emitting full sentences — they output a short Japanese noun phrase instead.
PROMPT="あなたはターミナル画面を見て、ユーザーが今何をしているかを推測するアシスタントです。
以下のターミナル内容から、作業内容を日本語の短い名詞句（10〜18文字）で1つだけ出力してください。文や句読点や引用符や説明は禁止。名詞句のみ。
例:
- cargoのビルド出力 → Rustのビルド
- git log → コミット履歴の確認
- npm test → テスト実行中

ターミナル内容:
$ZELLIJ_SIDEBAR_PANE_CONTENT"

# Build the JSON request body safely (jq escapes the prompt).
REQ=$(jq -nc --arg m "$MODEL" --arg p "$PROMPT" \
    '{model:$m, prompt:$p, stream:false, options:{num_predict:24, temperature:0.2}}')

SUMMARY=$(printf '%s' "$REQ" | curl -s --max-time 30 "$HOST/api/generate" -d @- 2>/dev/null \
    | jq -r '.response // empty' | tr -s ' \n' ' ' | sed 's/^ //;s/ $//')

# Guard: refusals/explanations are long; 不明 is the explicit sentinel.
case "$SUMMARY" in
    "" | "不明") exit 0 ;;
esac
[ "${#SUMMARY}" -le 90 ] || exit 0  # ~30 JP chars in UTF-8 bytes

printf '%s %s\n' "$HASH" "$SUMMARY" > "$CACHE"
printf '%s\n' "$SUMMARY"
