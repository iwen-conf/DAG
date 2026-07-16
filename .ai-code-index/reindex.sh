#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
INDEX_DIR="${SCRIPT_DIR}/index"
TAGS_FILE="${SCRIPT_DIR}/tags"

# 生成索引时复用本地搜索工具的精选路径集，避免旧项目和参考项目混入默认结果。
# Zoekt 的 -ignore_dirs 按目录名匹配，因此这里显式传入 profile 路径来控制边界。
source "${SCRIPT_DIR}/lib.sh"
ensure_search_path

PROFILE="${DEFAULT_PROFILE}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="$2"
      shift 2
      ;;
    --profile=*)
      PROFILE="${1#--profile=}"
      shift
      ;;
    -h|--help)
      cat <<'EOF'
usage: .ai-code-index/reindex.sh [--profile code|rust|docs|docs-full|meta|all|ref]

按指定 profile 重建 Zoekt 分片和 Universal Ctags 符号索引。
默认 profile：code（Rust 源码若存在 + 需求分析文档 + 元信息）。
EOF
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      echo "用法: .ai-code-index/reindex.sh [--profile <name>]" >&2
      exit 2
      ;;
  esac
done

PROFILE="$(normalize_profile "${PROFILE}")"
require_cmd zoekt-index
require_cmd ctags

if ! ctags --version 2>/dev/null | head -1 | grep -qi "Universal Ctags"; then
  echo "error: 需要 Universal Ctags；当前 ctags 不兼容" >&2
  exit 127
fi

IGNORE_DIRS="${AI_CODE_INDEX_IGNORE_DIRS:-${DEFAULT_IGNORE_DIRS}}"

INDEX_PATHS=()
while IFS= read -r path; do
  INDEX_PATHS+=("${path}")
done < <(existing_profile_paths "${PROFILE}")
if [[ ${#INDEX_PATHS[@]} -eq 0 ]]; then
  echo "error: profile '${PROFILE}' 没有可索引路径" >&2
  exit 2
fi

START_TS="$(date +%s)"
echo "profile: ${PROFILE}"
echo "索引路径 (${#INDEX_PATHS[@]}):"
for path in "${INDEX_PATHS[@]}"; do
  count="$(count_files_under "${ROOT_DIR}/${path}")"
  printf '  - %s (%s files)\n' "${path}" "${count}"
done

mkdir -p "${INDEX_DIR}"
rm -f "${INDEX_DIR}"/*.zoekt
rm -f "${INDEX_DIR}"/*.zoekt.*.tmp

cd "${ROOT_DIR}"

for path in "${INDEX_PATHS[@]}"; do
  echo "zoekt-index: ${path}"
  zoekt-index \
    -index "${INDEX_DIR}" \
    -ignore_dirs "${IGNORE_DIRS}" \
    -parallelism "${ZOEKT_PARALLELISM:-4}" \
    -shard_prefix_override "$(shard_prefix_for_path "${path}")" \
    "${path}"
done

CTAGS_EXCLUDES=()
while IFS= read -r flag; do
  CTAGS_EXCLUDES+=("${flag}")
done < <(ctags_exclude_args)

echo "ctags: ${#INDEX_PATHS[@]} 个路径"
ctags \
  --recurse=yes \
  --langmap=TypeScript:+.ets \
  --fields=+n+K+S+l \
  "${CTAGS_EXCLUDES[@]}" \
  -f "${TAGS_FILE}" \
  "${INDEX_PATHS[@]}"

END_TS="$(date +%s)"
ELAPSED="$((END_TS - START_TS))"
SHARD_COUNT="$(find "${INDEX_DIR}" -maxdepth 1 -name '*.zoekt' | wc -l | tr -d ' ')"
SYMBOL_COUNT="$(awk 'BEGIN{FS="\t";c=0} $1!~/^!/{c++} END{print c+0}' "${TAGS_FILE}")"

# 按扩展名汇总符号数量，快速暴露“某类源码没有被索引”的问题。
EXT_SUMMARY="$(
  awk 'BEGIN{FS="\t"}
    $1 !~ /^!/ {
      f=$2
      n=split(f, a, ".")
      ext=(n>1 ? a[n] : "(none)")
      c[ext]++
    }
    END {
      for (e in c) printf "%d\t%s\n", c[e], e
    }' "${TAGS_FILE}" | sort -nr | head -12 | awk '{printf "%s:%s ", $2, $1}'
)"

printf 'profile_file=%s\n' "${SCRIPT_DIR}/.last-profile"
printf '%s\n' "${PROFILE}" >"${SCRIPT_DIR}/.last-profile"
date -u '+%Y-%m-%dT%H:%M:%SZ' >"${SCRIPT_DIR}/.last-indexed-at"

echo "----"
echo "Zoekt 索引: ${INDEX_DIR} (${SHARD_COUNT} 个分片)"
echo "Ctags 文件: ${TAGS_FILE} (${SYMBOL_COUNT} 个符号)"
echo "新鲜度标记: ${SCRIPT_DIR}/.last-indexed-at"
echo "主要符号扩展名: ${EXT_SUMMARY}"
echo "耗时: ${ELAPSED}s"
echo "完成。示例搜索："
echo "  .ai-code-index/search.sh --profile docs 'Claim'"
echo "  .ai-code-index/search.sh --profile code 'Task IR'"
echo "  .ai-code-index/search.sh --profile meta 'profile_path_list'"
echo "  .ai-code-index/symbols.sh Expand"
