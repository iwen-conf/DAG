#!/usr/bin/env bash
# 本地代码索引脚本的公共辅助函数（DAG 项目）。
# Profile 约定：
#   code      Rust 源码（若存在）+ 需求分析轻量文档 + 元信息（默认）
#   rust      仅 Rust 源码与测试（src / crates / tests / benches）
#   docs      稳定需求/设计文档（默认 docs/00-需求分析）
#   docs-full 完整 docs/ 树
#   meta      根配置、AGENTS.md、.ai-code-index 工具自身
#   all       code 范围 + 完整 docs/
#   ref       可选参考树（默认无路径；需要时再填）

set -euo pipefail

index_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${index_dir}/.." && pwd)"

DEFAULT_PROFILE="${AI_CODE_INDEX_PROFILE:-code}"
AUTO_REINDEX_DEFAULT="${AI_CODE_INDEX_AUTO_REINDEX:-1}"

# Rust 实现源码根（路径存在时才会进入索引）。
paths_rust=(
  "src"
  "crates"
  "tests"
  "benches"
  "examples"
  "Cargo.toml"
  "Cargo.lock"
)

# 稳定产品/需求文档（轻量）。
paths_docs_light=(
  "docs/00-需求分析"
  "docs/README.md"
)

paths_docs_full=(
  "docs"
)

paths_meta=(
  "README.md"
  ".gitignore"
  "AGENTS.md"
  "rust-toolchain.toml"
  "rustfmt.toml"
  "clippy.toml"
  ".ai-code-index/README.md"
  ".ai-code-index/lib.sh"
  ".ai-code-index/manifest.yaml"
  ".ai-code-index/files.sh"
  ".ai-code-index/stats.sh"
  ".ai-code-index/install-tools.sh"
  ".ai-code-index/reindex.sh"
  ".ai-code-index/repository-map.md"
  ".ai-code-index/search.sh"
  ".ai-code-index/struct-search.sh"
  ".ai-code-index/symbols.sh"
  ".ai-code-index/doctor.sh"
)

# 可选参考树；默认 code/all 不索引。
paths_ref=(
  "reference"
  "vendor"
)

# Zoekt -ignore_dirs 按目录名匹配。
DEFAULT_IGNORE_DIRS=".git,node_modules,dist,build,target,_release,.next,.nuxt,coverage,tmp,.tmp,vendor,.arc,.magi,.ace-tool,.venv,.cocoindex_code,.pytest_cache,.mypy_cache,.ruff_cache,__pycache__,runtime,uploads,dogfood-output,release,exports,.idea,.cargo,index"

rg_common_args=(
  "--hidden"
  "--line-number"
  "--column"
  "--smart-case"
  "--color=never"
  "--glob" "!/.git/**"
  "--glob" "!.DS_Store"
  "--glob" "!vendor/**"
  "--glob" "!node_modules/**"
  "--glob" "!tmp/**"
  "--glob" "!**/build/**"
  "--glob" "!**/target/**"
  "--glob" "!dist/**"
  "--glob" "!*.log"
  "--glob" "!.ai-code-index/index/**"
  "--glob" "!.ai-code-index/tags"
)

# struct-search 未显式传入路径时，按语言选择默认搜索根。
struct_default_paths_for() {
  local lang
  lang="$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')"
  case "${lang}" in
    rust|rs)
      printf '%s\n' "src" "crates" "tests" "benches" "examples"
      ;;
    markdown|md)
      printf '%s\n' "docs"
      ;;
    yaml|yml)
      printf '%s\n' ".ai-code-index" "docs"
      ;;
    bash|sh)
      printf '%s\n' ".ai-code-index"
      ;;
  esac
}

ensure_search_path() {
  if [[ -d "${HOME}/go/bin" ]]; then
    case ":${PATH}:" in
      *":${HOME}/go/bin:"*) ;;
      *) export PATH="${HOME}/go/bin:${PATH}" ;;
    esac
  fi
  if [[ -d "${HOME}/.cargo/bin" ]]; then
    case ":${PATH}:" in
      *":${HOME}/.cargo/bin:"*) ;;
      *) export PATH="${HOME}/.cargo/bin:${PATH}" ;;
    esac
  fi
  local mise_go_bin
  for mise_go_bin in "${HOME}/.local/share/mise/installs/go"/*/bin; do
    if [[ -d "${mise_go_bin}" ]]; then
      case ":${PATH}:" in
        *":${mise_go_bin}:"*) ;;
        *) export PATH="${mise_go_bin}:${PATH}" ;;
      esac
    fi
  done
}

normalize_profile() {
  local profile="${1:-${DEFAULT_PROFILE}}"
  profile="$(printf '%s' "${profile}" | tr '[:upper:]' '[:lower:]')"
  case "${profile}" in
    code|rust|docs|docs-full|meta|all|ref) printf '%s\n' "${profile}" ;;
    backend)
      # 兼容 skill 文档里的 backend 命名；本仓库映射到 rust。
      printf '%s\n' "rust"
      ;;
    *)
      echo "error: 未知 profile '${profile}'（允许：code|rust|docs|docs-full|meta|all|ref；backend→rust）" >&2
      return 2
      ;;
  esac
}

profile_zoekt_filter() {
  local profile
  profile="$(normalize_profile "${1:-}")" || return $?
  case "${profile}" in
    rust) printf 'r:^(src|crates|tests|benches|examples|Cargo\\.toml|Cargo\\.lock)$' ;;
    # zoekt-index 以索引路径 basename 作为 repository 名。
    docs) printf 'r:^(00-需求分析)$' ;;
    docs-full) printf 'r:^(docs|00-需求分析)$' ;;
    meta) printf 'r:^(README\\.md|\\.gitignore|AGENTS\\.md|rust-toolchain\\.toml|rustfmt\\.toml|clippy\\.toml|lib\\.sh|manifest\\.yaml|files\\.sh|stats\\.sh|install-tools\\.sh|reindex\\.sh|repository-map\\.md|search\\.sh|struct-search\\.sh|symbols\\.sh|doctor\\.sh)$' ;;
    ref) printf 'r:^(reference|vendor)$' ;;
    code|all) printf '' ;;
  esac
}

profile_shard_globs() {
  local profile
  profile="$(normalize_profile "${1:-}")" || return $?
  case "${profile}" in
    rust) printf '%s\n' "src_*" "crates_*" "tests_*" "benches_*" "examples_*" "Cargo_*" ;;
    docs) printf '%s\n' "docs_*" "00_*" ;;
    docs-full) printf '%s\n' "docs_*" ;;
    meta) printf '%s\n' "README_*" "AGENTS_*" "gitignore_*" "ai_code_index_*" "rust_*" "clippy_*" ;;
    ref) printf '%s\n' "reference_*" "vendor_*" ;;
    code|all) printf '' ;;
  esac
}

profile_path_list() {
  local profile
  profile="$(normalize_profile "${1:-}")" || return $?
  local paths=()

  case "${profile}" in
    rust)
      paths+=("${paths_rust[@]}")
      ;;
    docs)
      paths+=("${paths_docs_light[@]}")
      ;;
    docs-full)
      paths+=("${paths_docs_full[@]}")
      ;;
    meta)
      paths+=("${paths_meta[@]}")
      ;;
    ref)
      paths+=("${paths_ref[@]}")
      ;;
    code)
      paths+=("${paths_rust[@]}")
      paths+=("${paths_docs_light[@]}")
      paths+=("${paths_meta[@]}")
      ;;
    all)
      paths+=("${paths_rust[@]}")
      paths+=("${paths_docs_full[@]}")
      paths+=("${paths_meta[@]}")
      ;;
  esac

  local path
  for path in "${paths[@]}"; do
    printf '%s\n' "${path}"
  done
}

existing_profile_paths() {
  local profile="${1:-${DEFAULT_PROFILE}}"
  local path
  while IFS= read -r path; do
    if [[ -e "${repo_root}/${path}" ]]; then
      printf '%s\n' "${path}"
    fi
  done < <(profile_path_list "${profile}")
}

existing_repo_paths() {
  existing_profile_paths "${DEFAULT_PROFILE}"
}

shard_prefix_for_path() {
  local path="$1"
  path="${path#./}"
  path="${path//[^A-Za-z0-9]/_}"
  path="${path##_}"
  path="${path%%_}"
  if [[ -z "${path}" ]]; then
    path="root"
  fi
  printf '%s\n' "${path}"
}

last_index_profile() {
  if [[ -f "${index_dir}/.last-profile" ]]; then
    tr -d '[:space:]' <"${index_dir}/.last-profile"
  fi
}

profile_covers() {
  local requested="${1:-${DEFAULT_PROFILE}}"
  local built="${2:-}"
  requested="$(normalize_profile "${requested}")" || return $?
  if [[ -z "${built}" ]]; then
    return 1
  fi
  built="$(normalize_profile "${built}")" || return $?

  case "${built}" in
    all)
      case "${requested}" in
        ref) return 1 ;;
        *) return 0 ;;
      esac
      ;;
    code)
      case "${requested}" in
        code|rust|docs|meta) return 0 ;;
        *) return 1 ;;
      esac
      ;;
    docs-full)
      case "${requested}" in
        docs|docs-full) return 0 ;;
        *) return 1 ;;
      esac
      ;;
    *)
      [[ "${requested}" == "${built}" ]]
      ;;
  esac
}

index_refresh_profile_for() {
  local requested="${1:-${DEFAULT_PROFILE}}"
  requested="$(normalize_profile "${requested}")" || return $?
  local built
  built="$(last_index_profile)"
  if profile_covers "${requested}" "${built}"; then
    printf '%s\n' "${built}"
  else
    printf '%s\n' "${requested}"
  fi
}

profile_expected_shard_prefixes() {
  local profile="${1:-${DEFAULT_PROFILE}}"
  local path
  while IFS= read -r path; do
    shard_prefix_for_path "${path}"
  done < <(existing_profile_paths "${profile}")
}

prefix_in_list() {
  local needle="$1"
  shift || true
  local item
  for item in "$@"; do
    if [[ "${needle}" == "${item}" ]]; then
      return 0
    fi
  done
  return 1
}

unexpected_shard_prefixes() {
  local profile="${1:-${DEFAULT_PROFILE}}"
  local expected=()
  local prefix shard base
  while IFS= read -r prefix; do
    [[ -n "${prefix}" ]] && expected+=("${prefix}")
  done < <(profile_expected_shard_prefixes "${profile}")

  if [[ ! -d "${index_dir}/index" ]]; then
    return 0
  fi
  for shard in "${index_dir}/index/"*.zoekt; do
    [[ -e "${shard}" ]] || continue
    base="$(basename "${shard}")"
    prefix="${base%%_v*}"
    if ! prefix_in_list "${prefix}" "${expected[@]}"; then
      printf '%s\n' "${prefix}"
    fi
  done | sort -u
}

index_control_paths() {
  printf '%s\n' \
    ".ai-code-index/README.md" \
    ".ai-code-index/lib.sh" \
    ".ai-code-index/manifest.yaml" \
    ".ai-code-index/files.sh" \
    ".ai-code-index/stats.sh" \
    ".ai-code-index/install-tools.sh" \
    ".ai-code-index/reindex.sh" \
    ".ai-code-index/repository-map.md" \
    ".ai-code-index/search.sh" \
    ".ai-code-index/struct-search.sh" \
    ".ai-code-index/symbols.sh" \
    ".ai-code-index/doctor.sh" \
    "AGENTS.md"
}

find_newer_index_input_under() {
  local rel_path="$1"
  local marker="$2"
  local root="${repo_root}/${rel_path}"
  if [[ ! -e "${root}" ]]; then
    return 0
  fi
  if [[ -f "${root}" ]]; then
    if [[ "${root}" -nt "${marker}" ]]; then
      printf '%s\n' "${rel_path}"
    fi
    return 0
  fi

  find "${root}" \
    \( -type d \( \
      -name .git -o \
      -name node_modules -o \
      -name dist -o \
      -name build -o \
      -name target -o \
      -name _release -o \
      -name .next -o \
      -name .nuxt -o \
      -name coverage -o \
      -name tmp -o \
      -name .tmp -o \
      -name vendor -o \
      -name .arc -o \
      -name .magi -o \
      -name .ace-tool -o \
      -name .venv -o \
      -name .cocoindex_code -o \
      -name .pytest_cache -o \
      -name .mypy_cache -o \
      -name .ruff_cache -o \
      -name __pycache__ -o \
      -name runtime -o \
      -name uploads -o \
      -name dogfood-output -o \
      -name release -o \
      -name exports -o \
      -name .idea -o \
      -name index \
    \) -prune \) -o \
    \( -type f \
      ! -name '*.log' \
      -newer "${marker}" \
      -print -quit \
    \)
}

newer_index_inputs() {
  local profile="${1:-${DEFAULT_PROFILE}}"
  local marker="$2"
  local path
  while IFS= read -r path; do
    find_newer_index_input_under "${path}" "${marker}"
  done < <(existing_profile_paths "${profile}")
  while IFS= read -r path; do
    find_newer_index_input_under "${path}" "${marker}"
  done < <(index_control_paths)
}

index_exclude_names() {
  local IFS=',' parts item
  read -r -a parts <<<"${DEFAULT_IGNORE_DIRS}"
  for item in "${parts[@]}"; do
    [[ -n "${item}" ]] && printf '%s\n' "${item}"
  done
}

index_exclude_csv() {
  index_exclude_names | paste -sd ',' -
}

fd_command() {
  if command -v fd >/dev/null 2>&1; then
    command -v fd
  elif command -v fdfind >/dev/null 2>&1; then
    command -v fdfind
  else
    return 1
  fi
}

acquire_reindex_lock() {
  local lock_dir="${index_dir}/.reindex.lock"
  local waited=0
  local max_wait="${AI_CODE_INDEX_LOCK_WAIT_SECONDS:-120}"
  while ! mkdir "${lock_dir}" 2>/dev/null; do
    if [[ "${waited}" -ge "${max_wait}" ]]; then
      echo "error: 等待 .ai-code-index 自动重建锁超时：${lock_dir}" >&2
      return 1
    fi
    sleep 1
    waited=$((waited + 1))
  done
  printf '%s\n' "$$" >"${lock_dir}/pid"
}

release_reindex_lock() {
  rm -rf "${index_dir}/.reindex.lock"
}

index_needs_refresh() {
  local requested="${1:-${DEFAULT_PROFILE}}"
  local target="${2:-${requested}}"
  requested="$(normalize_profile "${requested}")" || return $?
  target="$(normalize_profile "${target}")" || return $?

  if ! compgen -G "${index_dir}/index/*.zoekt" >/dev/null; then
    return 0
  fi
  if [[ ! -f "${index_dir}/tags" ]]; then
    return 0
  fi

  local built
  built="$(last_index_profile)"
  if ! profile_covers "${requested}" "${built}"; then
    return 0
  fi
  if [[ "${built}" != "${target}" ]]; then
    return 0
  fi

  local marker="${index_dir}/.last-indexed-at"
  if [[ ! -f "${marker}" ]]; then
    return 0
  fi

  if [[ -n "$(unexpected_shard_prefixes "${target}")" ]]; then
    return 0
  fi
  if [[ -n "$(newer_index_inputs "${target}" "${marker}")" ]]; then
    return 0
  fi

  return 1
}

ensure_index_fresh() {
  local requested="${1:-${DEFAULT_PROFILE}}"
  requested="$(normalize_profile "${requested}")" || return $?

  case "${AUTO_REINDEX_DEFAULT}" in
    0|false|FALSE|no|NO)
      return 0
      ;;
  esac

  local target
  target="$(index_refresh_profile_for "${requested}")"
  if index_needs_refresh "${requested}" "${target}"; then
    acquire_reindex_lock
    target="$(index_refresh_profile_for "${requested}")"
    if index_needs_refresh "${requested}" "${target}"; then
      echo "info: .ai-code-index 索引缺失或已过期，自动重建 profile '${target}'（禁用：AI_CODE_INDEX_AUTO_REINDEX=0）" >&2
      if ! "${index_dir}/reindex.sh" --profile "${target}" >&2; then
        release_reindex_lock
        return 1
      fi
    fi
    release_reindex_lock
  fi
}

require_rg() {
  if ! command -v rg >/dev/null 2>&1; then
    echo "error: .ai-code-index 脚本需要 ripgrep (rg)" >&2
    exit 127
  fi
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: 必需命令不存在或不在 PATH 中：$1" >&2
    exit 127
  fi
}

ctags_exclude_args() {
  local excludes=(
    .git
    node_modules
    dist
    build
    target
    _release
    .next
    .nuxt
    coverage
    tmp
    .tmp
    vendor
    .ai-code-index/index
    .arc
    .magi
    .ace-tool
    .venv
    .cocoindex_code
    .pytest_cache
    .mypy_cache
    .ruff_cache
    __pycache__
    runtime
    uploads
    dogfood-output
    release
    exports
    .idea
    index
  )
  local item
  for item in "${excludes[@]}"; do
    printf -- '--exclude=%s\n' "${item}"
  done
}

count_files_under() {
  local root="$1"
  if [[ ! -e "${root}" ]]; then
    printf '0'
    return
  fi
  if [[ -f "${root}" ]]; then
    printf '1'
    return
  fi
  find "${root}" -type f \
    ! -path '*/.git/*' \
    ! -path '*/node_modules/*' \
    ! -path '*/vendor/*' \
    ! -path '*/build/*' \
    ! -path '*/target/*' \
    ! -path '*/dist/*' \
    ! -path '*/.ai-code-index/index/*' \
    2>/dev/null | wc -l | tr -d ' '
}

run_rg() {
  require_rg
  if [[ $# -eq 0 ]]; then
    echo "error: run_rg 需要搜索模式" >&2
    exit 2
  fi

  local profile="${DEFAULT_PROFILE}"
  local pattern=""
  local rg_args=()
  local requested_paths=()

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --profile)
        profile="$2"
        shift 2
        ;;
      --profile=*)
        profile="${1#--profile=}"
        shift
        ;;
      --)
        shift
        break
        ;;
      -*)
        rg_args+=("$1")
        shift
        ;;
      *)
        if [[ -z "${pattern}" ]]; then
          pattern="$1"
        elif [[ -e "${repo_root}/$1" ]]; then
          requested_paths+=("$1")
        else
          rg_args+=("$1")
        fi
        shift
        ;;
    esac
  done

  while [[ $# -gt 0 ]]; do
    if [[ -e "${repo_root}/$1" ]]; then
      requested_paths+=("$1")
    else
      rg_args+=("$1")
    fi
    shift
  done

  if [[ -z "${pattern}" ]]; then
    echo "error: run_rg 需要搜索模式" >&2
    exit 2
  fi

  local paths=()
  if [[ ${#requested_paths[@]} -gt 0 ]]; then
    paths=("${requested_paths[@]}")
  else
    while IFS= read -r path; do
      paths+=("${path}")
    done < <(existing_profile_paths "${profile}")
  fi

  if [[ ${#paths[@]} -eq 0 ]]; then
    echo "error: profile '${profile}' 没有可搜索路径" >&2
    exit 2
  fi

  local abs_paths=()
  local p
  for p in "${paths[@]}"; do
    abs_paths+=("${repo_root}/${p}")
  done

  (
    cd "${repo_root}"
    rg "${rg_common_args[@]}" "${rg_args[@]}" -- "${pattern}" "${abs_paths[@]}"
  )
}
