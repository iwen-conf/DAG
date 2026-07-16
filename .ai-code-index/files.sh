#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
source "${SCRIPT_DIR}/lib.sh"

usage() {
  cat <<'EOF'
usage: .ai-code-index/files.sh [--profile code|rust|docs|docs-full|meta|all|ref] [--ext EXT] [pattern]

Profile-aware file discovery. Prefers Rust fd/fdfind and falls back to POSIX find.

Examples:
  .ai-code-index/files.sh --profile docs --ext md '状态'
  .ai-code-index/files.sh --profile rust --ext rs 'task'
  .ai-code-index/files.sh --profile meta 'yaml|toml|yml'
EOF
}

PROFILE="${DEFAULT_PROFILE}"
EXT=""
PATTERN=""

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
    --ext|-e)
      EXT="${2#.}"
      shift 2
      ;;
    --ext=*)
      EXT="${1#--ext=}"
      EXT="${EXT#.}"
      shift
      ;;
    -*)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -z "${PATTERN}" ]]; then
        PATTERN="$1"
      else
        PATTERN="${PATTERN}|$1"
      fi
      shift
      ;;
  esac
done

PROFILE="$(normalize_profile "${PROFILE}")"
PATHS=()
while IFS= read -r path; do
  PATHS+=("${path}")
done < <(existing_profile_paths "${PROFILE}")

if [[ ${#PATHS[@]} -eq 0 ]]; then
  echo "error: profile '${PROFILE}' 没有可搜索路径" >&2
  exit 2
fi

cd "${ROOT_DIR}"

DIR_PATHS=()
FILE_PATHS=()
for path in "${PATHS[@]}"; do
  if [[ -d "${path}" ]]; then
    DIR_PATHS+=("${path}")
  elif [[ -f "${path}" ]]; then
    FILE_PATHS+=("${path}")
  fi
done

emit_matching_file_path() {
  local path="$1"
  if [[ -n "${EXT}" ]]; then
    case "${path}" in
      *."${EXT}") ;;
      *) return 0 ;;
    esac
  fi
  if [[ -n "${PATTERN}" && ! "${path}" =~ ${PATTERN} ]]; then
    return 0
  fi
  printf '%s\n' "${path}"
}

if FD_BIN="$(fd_command 2>/dev/null)"; then
  FD_ARGS=("--hidden" "--type" "f" "--full-path" "--color" "never")
  while IFS= read -r exclude; do
    FD_ARGS+=("--exclude" "${exclude}")
  done < <(index_exclude_names)
  if [[ -n "${EXT}" ]]; then
    FD_ARGS+=("--extension" "${EXT}")
  fi
  if [[ ${#DIR_PATHS[@]} -gt 0 ]]; then
    "${FD_BIN}" "${FD_ARGS[@]}" "${PATTERN:-.}" "${DIR_PATHS[@]}"
  fi
  if [[ ${#FILE_PATHS[@]} -gt 0 ]]; then
    for path in "${FILE_PATHS[@]}"; do
      emit_matching_file_path "${path}"
    done
  fi
  exit 0
fi

find_file_args=()
if [[ ${#DIR_PATHS[@]} -gt 0 ]]; then
  for path in "${DIR_PATHS[@]}"; do
    find_file_args+=("${path}")
  done
fi

if [[ ${#find_file_args[@]} -gt 0 ]]; then
  find "${find_file_args[@]}" \
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
      -name .gradle -o \
      -name .hvigor -o \
      -name .preview -o \
      -name .idea -o \
      -name .kotlin -o \
      -name signing -o \
      -name results \
    \) -prune \) -o \
    \( -type f \
      ! -name server \
      ! -name '*.log' \
      -print \
    \) | awk -v pattern="${PATTERN}" -v ext="${EXT}" '
      pattern != "" && $0 !~ pattern { next }
      ext != "" && $0 !~ ("\\." ext "$") { next }
      { print }
    '
fi
if [[ ${#FILE_PATHS[@]} -gt 0 ]]; then
  for path in "${FILE_PATHS[@]}"; do
    emit_matching_file_path "${path}"
  done
fi
