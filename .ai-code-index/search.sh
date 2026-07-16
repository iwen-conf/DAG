#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INDEX_DIR="${SCRIPT_DIR}/index"
source "${SCRIPT_DIR}/lib.sh"
ensure_search_path

PROFILE=""
QUERY_ARGS=()

usage() {
  cat <<'EOF'
usage: .ai-code-index/search.sh [--profile code|rust|docs|docs-full|meta|all|ref] <zoekt-query> [zoekt-options...]

示例：
  .ai-code-index/search.sh 'Claim'
  .ai-code-index/search.sh --profile docs 'Package'
  .ai-code-index/search.sh --profile rust 'fn main'
  .ai-code-index/search.sh --profile meta 'normalize_profile'

默认会在索引缺失或过期时自动重建；设置 AI_CODE_INDEX_AUTO_REINDEX=0 可禁用。
EOF
}

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
    --)
      shift
      QUERY_ARGS+=("$@")
      break
      ;;
    *)
      QUERY_ARGS+=("$@")
      break
      ;;
  esac
done

if [[ ${#QUERY_ARGS[@]} -lt 1 ]]; then
  usage >&2
  exit 2
fi

if ! command -v zoekt >/dev/null 2>&1; then
  echo "error: zoekt 未安装或不在 PATH 中（可尝试：export PATH=\"\$HOME/go/bin:\$PATH\"）" >&2
  exit 127
fi

REQUESTED_PROFILE="${PROFILE:-${DEFAULT_PROFILE}}"
ensure_index_fresh "${REQUESTED_PROFILE}"

if ! compgen -G "${INDEX_DIR}/*.zoekt" >/dev/null; then
  echo "error: Zoekt 索引不存在，请先运行 .ai-code-index/reindex.sh" >&2
  exit 1
fi

QUERY="${QUERY_ARGS[0]}"
EXTRA=("${QUERY_ARGS[@]:1}")

if [[ -n "${PROFILE}" ]]; then
  FILTER="$(profile_zoekt_filter "${PROFILE}" || true)"
  if [[ -n "${FILTER}" ]]; then
    # 在共享索引上追加 Zoekt repository 过滤，限制到指定 profile。
    QUERY="(${QUERY}) ${FILTER}"
  fi
fi

zoekt -r -index_dir "${INDEX_DIR}" "${QUERY}" ${EXTRA[@]+"${EXTRA[@]}"}
