#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
source "${SCRIPT_DIR}/lib.sh"

usage() {
  cat <<'EOF'
usage: .ai-code-index/stats.sh [--profile code|rust|docs|docs-full|meta|all|ref] [--json]

Profile-aware code inventory. Prefers Go scc, then Rust tokei, then a small shell fallback.

Examples:
  .ai-code-index/stats.sh --profile code
  .ai-code-index/stats.sh --profile backend --json
EOF
}

PROFILE="${DEFAULT_PROFILE}"
JSON=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --profile)
      PROFILE="$2"
      shift 2
      ;;
    --profile=*)
      PROFILE="${1#--profile=}"
      shift
      ;;
    --json)
      JSON=1
      shift
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

PROFILE="$(normalize_profile "${PROFILE}")"
PATHS=()
while IFS= read -r path; do
  PATHS+=("${path}")
done < <(existing_profile_paths "${PROFILE}")

if [[ ${#PATHS[@]} -eq 0 ]]; then
  echo "error: profile '${PROFILE}' 没有可统计路径" >&2
  exit 2
fi

cd "${ROOT_DIR}"

if command -v scc >/dev/null 2>&1; then
  if [[ "${JSON}" -eq 1 ]]; then
    scc --format json --exclude-dir "$(index_exclude_csv)" "${PATHS[@]}"
  else
    scc --no-cocomo --exclude-dir "$(index_exclude_csv)" "${PATHS[@]}"
  fi
  exit 0
fi

if command -v tokei >/dev/null 2>&1; then
  TOKEI_ARGS=("--hidden")
  while IFS= read -r exclude; do
    TOKEI_ARGS+=("--exclude" "${exclude}")
  done < <(index_exclude_names)
  if [[ "${JSON}" -eq 1 ]]; then
    TOKEI_ARGS+=("--output" "json")
  fi
  tokei "${TOKEI_ARGS[@]}" "${PATHS[@]}"
  exit 0
fi

if [[ "${JSON}" -eq 1 ]]; then
  echo "error: JSON stats require Go scc or Rust tokei" >&2
  exit 127
fi

"${SCRIPT_DIR}/files.sh" --profile "${PROFILE}" | awk '
  {
    ext = "(none)"
    n = split($0, parts, ".")
    if (n > 1) ext = parts[n]
    files[ext]++
  }
  END {
    printf "%-16s %8s\n", "extension", "files"
    printf "%-16s %8s\n", "---------", "-----"
    for (ext in files) printf "%-16s %8d\n", ext, files[ext]
  }
' | sort
