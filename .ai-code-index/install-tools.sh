#!/usr/bin/env bash
set -euo pipefail

CORE_BREW_PACKAGES=(
  fd
  scc
  yq
  dasel
  tokei
  jaq
  sd
)

usage() {
  cat <<'EOF'
usage: .ai-code-index/install-tools.sh [--dry-run]

Installs the project code-index helper CLI set using Homebrew only.
All listed tools are Rust or Go implementations; no Python/TypeScript CLIs are installed.

Core tools:
  fd      Rust  profile-aware file discovery
  scc     Go    code inventory/statistics
  yq      Go    YAML/JSON querying
  dasel   Go    JSON/YAML/TOML querying
  tokei   Rust  fallback code statistics
  jaq     Rust  jq-compatible JSON queries
  sd      Rust  safer stream replacements
EOF
}

DRY_RUN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if ! command -v brew >/dev/null 2>&1; then
  echo "error: Homebrew is required for this project helper installer" >&2
  exit 127
fi

for package in "${CORE_BREW_PACKAGES[@]}"; do
  if brew list --versions "${package}" >/dev/null 2>&1; then
    printf '[ok] %s already installed\n' "${package}"
    continue
  fi
  if [[ "${DRY_RUN}" -eq 1 ]]; then
    printf '[dry-run] brew install %s\n' "${package}"
  else
    brew install "${package}"
  fi
done
