#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
source "${SCRIPT_DIR}/lib.sh"

usage() {
  cat <<'EOF'
usage: .ai-code-index/struct-search.sh <language> '<pattern>' [paths...]

未传 paths 时，会按语言搜索默认源码根：
  go/kotlin/arkts/typescript/js/html/css

示例：
  .ai-code-index/struct-search.sh go 'if err != nil { $$$ }'
  .ai-code-index/struct-search.sh go 'func registerNovelRoutes($$$) { $$$ }' backend/internal/interface/restful/router/routes
  .ai-code-index/struct-search.sh kotlin 'class $NAME'
  .ai-code-index/struct-search.sh arkts 'class $NAME'
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [ "$#" -lt 2 ]; then
  usage >&2
  exit 2
fi

if ! command -v sg >/dev/null 2>&1; then
  echo "error: ast-grep CLI 'sg' 未安装或不在 PATH 中" >&2
  exit 127
fi

LANGUAGE="$1"
PATTERN="$2"
shift 2

cd "${ROOT_DIR}"

if [ "$#" -eq 0 ]; then
  DEFAULTS=()
  while IFS= read -r default_path; do
    [[ -n "${default_path}" ]] && DEFAULTS+=("${default_path}")
  done < <(struct_default_paths_for "${LANGUAGE}")
  if [[ ${#DEFAULTS[@]} -eq 0 ]]; then
    echo "error: language '${LANGUAGE}' 没有默认路径，请显式传入 paths" >&2
    exit 2
  fi
  set -- "${DEFAULTS[@]}"
fi

# 丢弃不存在的默认路径，避免可选源码树缺失时直接让 sg 失败。
EXISTING=()
for path in "$@"; do
  if [[ -e "${path}" ]]; then
    EXISTING+=("${path}")
  fi
done

if [[ ${#EXISTING[@]} -eq 0 ]]; then
  echo "error: 搜索路径均不存在：$*" >&2
  exit 2
fi

sg run --lang "${LANGUAGE}" --pattern "${PATTERN}" "${EXISTING[@]}"
