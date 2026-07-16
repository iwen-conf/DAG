#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
INDEX_DIR="${SCRIPT_DIR}/index"
TAGS_FILE="${SCRIPT_DIR}/tags"

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
      echo "usage: .ai-code-index/doctor.sh [--profile code|rust|docs|docs-full|meta|all|ref]"
      echo "检查本地代码索引工具链、profile 路径、生成索引和基础查询是否可用。"
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

PROFILE="$(normalize_profile "${PROFILE}")"
PASS=0
FAIL=0
WARN=0

ok() { echo "  [ok]  $*"; PASS=$((PASS + 1)); }
bad() { echo "  [FAIL] $*"; FAIL=$((FAIL + 1)); }
warn() { echo "  [warn] $*"; WARN=$((WARN + 1)); }

echo "== 工具链 =="
for cmd in zoekt zoekt-index ctags; do
  if command -v "${cmd}" >/dev/null 2>&1; then
    ok "${cmd}: $(command -v "${cmd}")"
  else
    bad "${cmd}: PATH 中未找到"
  fi
done
if command -v sg >/dev/null 2>&1; then
  ok "sg (ast-grep): $(command -v sg)"
else
  warn "sg (ast-grep): 未找到，struct-search 不可用"
fi
if command -v rg >/dev/null 2>&1; then
  ok "rg: $(command -v rg)"
else
  warn "rg: 未找到，run_rg 辅助函数不可用"
fi
if ctags --version 2>/dev/null | head -1 | grep -qi "Universal Ctags"; then
  ok "ctags 是 Universal Ctags"
else
  bad "ctags 不是 Universal Ctags"
fi

echo
echo "== Rust/Go 辅助 CLI =="
if FD_BIN="$(fd_command 2>/dev/null)"; then
  ok "fd/fdfind (Rust): ${FD_BIN}"
else
  warn "fd/fdfind (Rust): 未找到，files.sh 会退回 find"
fi
for spec in \
  "scc|Go|stats.sh 主统计器" \
  "yq|Go|YAML/JSON 查询" \
  "dasel|Go|JSON/YAML/TOML 查询" \
  "tokei|Rust|stats.sh 备用统计器" \
  "jaq|Rust|JSON 查询" \
  "sd|Rust|流式替换"
do
  IFS='|' read -r cmd lang purpose <<<"${spec}"
  if command -v "${cmd}" >/dev/null 2>&1; then
    ok "${cmd} (${lang}): $(command -v "${cmd}") - ${purpose}"
  else
    warn "${cmd} (${lang}): 未找到 - ${purpose}"
  fi
done

echo
echo "== profile 路径 (${PROFILE}) =="
PATHS=()
while IFS= read -r path; do
  PATHS+=("${path}")
done < <(existing_profile_paths "${PROFILE}")

if [[ ${#PATHS[@]} -eq 0 ]]; then
  bad "profile ${PROFILE} 没有实际存在的路径"
else
  ok "${#PATHS[@]} 个路径可在 ${ROOT_DIR} 下解析"
fi

HAS_DOCS=0
HAS_RUST=0
HAS_META=0
for path in "${PATHS[@]}"; do
  count="$(count_files_under "${ROOT_DIR}/${path}")"
  printf '  - %s (%s files)\n' "${path}" "${count}"
  case "${path}" in
    docs|docs/*) HAS_DOCS=1 ;;
    src|crates|tests|benches|examples|Cargo.toml|Cargo.lock) HAS_RUST=1 ;;
    .ai-code-index/*|AGENTS.md|README.md|.gitignore) HAS_META=1 ;;
  esac
done

if [[ "${PROFILE}" == "docs" || "${PROFILE}" == "docs-full" || "${PROFILE}" == "code" || "${PROFILE}" == "all" ]]; then
  if [[ "${HAS_DOCS}" -eq 1 ]]; then
    ok "已包含 docs 路径"
  else
    bad "profile ${PROFILE} 缺少 docs 路径"
  fi
fi
if [[ "${PROFILE}" == "rust" ]]; then
  if [[ "${HAS_RUST}" -eq 1 ]]; then
    ok "已包含 rust 源码路径"
  else
    warn "profile rust 尚无 src/crates/Cargo.toml（仓库仍处需求阶段属正常）"
  fi
fi
if [[ "${PROFILE}" == "code" || "${PROFILE}" == "all" ]]; then
  if [[ "${HAS_RUST}" -eq 0 ]]; then
    warn "code/all 尚未包含 Rust 源码路径（实现落地后会自动纳入）"
  else
    ok "已包含 rust 源码路径"
  fi
fi
if [[ "${PROFILE}" == "meta" || "${PROFILE}" == "code" || "${PROFILE}" == "all" ]]; then
  if [[ "${HAS_META}" -eq 1 ]]; then
    ok "已包含 meta 路径"
  else
    warn "profile ${PROFILE} 缺少 meta 路径"
  fi
fi

echo
echo "== 生成物 =="
if compgen -G "${INDEX_DIR}/*.zoekt" >/dev/null; then
  ok "Zoekt shards: $(ls -1 "${INDEX_DIR}"/*.zoekt 2>/dev/null | wc -l | tr -d ' ') 个"
else
  warn "无 Zoekt shards；运行 .ai-code-index/reindex.sh --profile ${PROFILE}"
fi
if [[ -f "${TAGS_FILE}" ]]; then
  ok "ctags tags: ${TAGS_FILE} ($(wc -l <"${TAGS_FILE}" | tr -d ' ') lines)"
else
  warn "无 tags 文件"
fi
if [[ -f "${SCRIPT_DIR}/.last-profile" ]]; then
  ok "last profile: $(tr -d '[:space:]' <"${SCRIPT_DIR}/.last-profile")"
else
  warn "无 .last-profile"
fi
if [[ -f "${SCRIPT_DIR}/.last-indexed-at" ]]; then
  ok "last indexed at marker present"
else
  warn "无 .last-indexed-at"
fi

BUILT="$(last_index_profile || true)"
if [[ -n "${BUILT}" ]]; then
  if profile_covers "${PROFILE}" "${BUILT}"; then
    ok "已有索引 profile '${BUILT}' 覆盖请求 '${PROFILE}'"
  else
    warn "已有索引 profile '${BUILT}' 不覆盖 '${PROFILE}'，搜索时会自动重建"
  fi
fi

if index_needs_refresh "${PROFILE}" "$(index_refresh_profile_for "${PROFILE}")"; then
  warn "索引判定为需要刷新"
else
  ok "索引新鲜度检查通过"
fi

echo
echo "== 冒烟查询 =="
if command -v zoekt >/dev/null 2>&1 && compgen -G "${INDEX_DIR}/*.zoekt" >/dev/null; then
  SMOKE_Q="Claim OR Package OR DAG"
  if ZOEK_OUT="$(zoekt -index_dir "${INDEX_DIR}" "${SMOKE_Q}" 2>/dev/null | head -5)"; then
    if [[ -n "${ZOEK_OUT}" ]]; then
      ok "zoekt smoke '${SMOKE_Q}' 有结果"
    else
      warn "zoekt smoke '${SMOKE_Q}' 无命中（文档内容变化时可能正常）"
    fi
  else
    warn "zoekt smoke 执行失败"
  fi
else
  warn "跳过 zoekt smoke（工具或 shards 缺失）"
fi

if [[ -f "${TAGS_FILE}" ]] && command -v rg >/dev/null 2>&1; then
  if rg -n "Claim|Package|Task" "${TAGS_FILE}" >/dev/null 2>&1; then
    ok "tags 中可见需求相关符号/词条"
  else
    warn "tags 中未匹配到 Claim/Package/Task（markdown 索引时可能较少符号）"
  fi
fi

echo
echo "== 汇总 =="
echo "  pass=${PASS} warn=${WARN} fail=${FAIL}"
if [[ "${FAIL}" -gt 0 ]]; then
  exit 1
fi
exit 0
