#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TAGS_FILE="${SCRIPT_DIR}/tags"
source "${SCRIPT_DIR}/lib.sh"

usage() {
  cat <<'EOF'
usage: .ai-code-index/symbols.sh [query] [--lang go|kotlin|arkts|ts|...] [--kind func|class|...] [--profile backend|android|harmony|pc|docs|...]

示例：
  .ai-code-index/symbols.sh RecordReadingProgress
  .ai-code-index/symbols.sh ReaderView --lang kotlin --kind class
  .ai-code-index/symbols.sh --profile backend ReadingProgress

默认会在索引缺失或过期时自动重建；设置 AI_CODE_INDEX_AUTO_REINDEX=0 可禁用。
EOF
}

QUERY=""
LANG_FILTER=""
KIND_FILTER=""
PROFILE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --lang)
      LANG_FILTER="$2"
      shift 2
      ;;
    --lang=*)
      LANG_FILTER="${1#--lang=}"
      shift
      ;;
    --kind)
      KIND_FILTER="$2"
      shift 2
      ;;
    --kind=*)
      KIND_FILTER="${1#--kind=}"
      shift
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
      if [[ $# -gt 0 && -z "${QUERY}" ]]; then
        QUERY="$*"
      fi
      break
      ;;
    -*)
      echo "error: 未知选项：$1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -z "${QUERY}" ]]; then
        QUERY="$1"
      else
        QUERY="${QUERY} $1"
      fi
      shift
      ;;
  esac
done

PATH_REGEX=""
if [[ -n "${PROFILE}" ]]; then
  case "$(normalize_profile "${PROFILE}")" in
    backend) PATH_REGEX="^backend/" ;;
    pc) PATH_REGEX="^front/PC/New/" ;;
    android) PATH_REGEX="^front/android/" ;;
    harmony) PATH_REGEX="^front/harmony/" ;;
    docs|docs-full) PATH_REGEX="^docs/" ;;
    meta) PATH_REGEX="^(scripts/|AGENTS\\.md|\\.ai-code-index/)" ;;
    ref) PATH_REGEX="^(reference_projects/|front/android/csxs-android/vendor/)" ;;
    code|all) PATH_REGEX="" ;;
  esac
fi

REQUESTED_PROFILE="${PROFILE:-${DEFAULT_PROFILE}}"
ensure_index_fresh "${REQUESTED_PROFILE}"

if [[ ! -f "${TAGS_FILE}" ]]; then
  echo "error: ctags 索引不存在，请先运行 .ai-code-index/reindex.sh" >&2
  exit 1
fi

# Universal Ctags 行格式：name<TAB>file<TAB>ex_cmd;"<TAB>kind:...<TAB>language:...
# 这里用 awk 直接解析 tags 文件，避免为简单符号查询再引入额外依赖。
awk -v q="${QUERY}" \
    -v lang="${LANG_FILTER}" \
    -v kind="${KIND_FILTER}" \
    -v pfx="${PATH_REGEX}" '
  BEGIN {
    FS = "\t"
    IGNORECASE = 1
    lang = tolower(lang)
    kind = tolower(kind)
  }
  $1 ~ /^!/ { next }
  {
    name = $1
    file = $2
    ex = $3
    k = ""
    l = ""
    for (i = 4; i <= NF; i++) {
      if ($i ~ /^kind:/) {
        k = $i
        sub(/^kind:/, "", k)
      } else if ($i ~ /^language:/) {
        l = $i
        sub(/^language:/, "", l)
      } else if (k == "" && $i !~ /:/) {
        # 部分 ctags 版本会把 kind 作为第 4 个裸字段输出。
        k = $i
      }
    }
    if (q != "" && name !~ q && file !~ q) next
    if (pfx != "" && file !~ pfx) next
    if (lang != "" && tolower(l) !~ lang && tolower(file) !~ ("\\." lang "$") && !(lang == "go" && file ~ /\.go$/) && !(lang ~ /^(kt|kotlin)$/ && file ~ /\.kt$/) && !(lang ~ /^(arkts|ets)$/ && file ~ /\.ets$/) && !(lang ~ /^(ts|typescript)$/ && file ~ /\.(tsx?|ets)$/) && !(lang ~ /^(js|javascript)$/ && file ~ /\.jsx?$/)) next
    if (kind != "" && tolower(k) !~ kind) next
    printf "%s\t%s\t%s\t%s\t%s\n", name, file, ex, k, l
  }
' "${TAGS_FILE}"
