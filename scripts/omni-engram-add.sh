#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $(basename "$0") <workspace> [engram_workspace] [topics]" >&2
  exit 2
fi

workspace="$1"
engram_workspace="${2:-$workspace}"
topics="${3:-${OMNI_TOPICS:-omni,context}}"
source="${OMNI_SOURCE:-omni-export}"
omni_bin="${OMNI_BIN:-omni}"
engram_bin="${ENGRAM_BIN:-engram}"
engram_cli="${ENGRAM_CLI:-}"
engram_node="${ENGRAM_NODE:-node}"

tmp_dir="${TMPDIR:-/tmp}"
tmp_json="${tmp_dir}/omni-export-$$.json"

if ! command -v "$omni_bin" >/dev/null 2>&1; then
  echo "omni binary not found: $omni_bin" >&2
  exit 1
fi

if ! command -v "$engram_bin" >/dev/null 2>&1; then
  if [[ -n "$engram_cli" ]]; then
    engram_bin="$engram_node"
  else
    echo "engram binary not found: $engram_bin" >&2
    echo "set ENGRAM_BIN or ENGRAM_CLI to continue" >&2
    exit 1
  fi
fi

"$omni_bin" export --workspace "$workspace" --format engram --json > "$tmp_json"

python3 - "$tmp_json" "$engram_bin" "$engram_workspace" "$topics" "$source" "$engram_cli" <<'PY'
import json
import subprocess
import sys

path, engram_bin, workspace, topics, source, engram_cli = sys.argv[1:]
with open(path, "r", encoding="utf-8") as f:
    payload = json.load(f)

content = None
if isinstance(payload, dict):
    if "export" in payload and isinstance(payload["export"], dict):
        content = payload["export"].get("content")
    else:
        content = payload.get("content")

if not content:
    raise SystemExit("omni export did not include content")

cmd = [engram_bin]
if engram_cli:
    cmd.append(engram_cli)
cmd.extend(["add", content, "--workspace", workspace, "--source", source])
if topics:
    cmd.extend(["--topics", topics])

subprocess.run(cmd, check=True)
PY

rm -f "$tmp_json"
