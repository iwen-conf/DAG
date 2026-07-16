# Local Code Index

Project-local search helpers for agents and humans. Local only — no remote indexing service.

Stack:

- **Zoekt** — full-text / regex search  
- **ast-grep (`sg`)** — structural (AST) search  
- **Universal Ctags** — symbol definitions  
- **fd / fdfind** — file discovery  
- **scc / tokei** — code inventory  
- **yq / dasel / jaq / sd** — config helpers  

## Profiles

Default: **`code`**.

| Profile | Contents |
| --- | --- |
| `code` | Rust sources (if present) + light docs + meta |
| `rust` | `src` / `crates` / `tests` / `benches` / `examples` / Cargo files |
| `docs` | `docs/00-需求分析` (+ `docs/README.md` if present) |
| `docs-full` | entire `docs/` |
| `meta` | root config, `AGENTS.md`, `.ai-code-index` scripts |
| `all` | rust + full docs + meta |
| `ref` | `reference` / `vendor` (on demand only) |

`backend` is accepted as an alias for `rust`.

Noise dirs (`target`, `node_modules`, `vendor`, `.git`, …) are ignored by name.

## Current repo state

This repository is **requirements-first**:

- Indexed today: `docs/00-需求分析/**`, `.ai-code-index/**`
- Reserved for later: `src`, `crates`, `tests`, Cargo workspace roots

When Rust code appears, re-run:

```bash
.ai-code-index/reindex.sh --profile code
```

## Commands

```bash
.ai-code-index/doctor.sh --profile code
.ai-code-index/reindex.sh --profile code
.ai-code-index/search.sh --profile docs "Claim"
.ai-code-index/search.sh --profile code "Task IR"
.ai-code-index/symbols.sh Expand --profile code
.ai-code-index/files.sh --profile docs --ext md '状态'
.ai-code-index/stats.sh --profile docs
.ai-code-index/struct-search.sh rust 'fn $NAME($$$)' src
.ai-code-index/install-tools.sh --dry-run
```

Freeze auto-refresh when needed:

```bash
AI_CODE_INDEX_AUTO_REINDEX=0 .ai-code-index/search.sh --profile docs "query"
```

## Generated (gitignored)

- `.ai-code-index/index/` — Zoekt shards  
- `.ai-code-index/tags` — ctags  
- `.ai-code-index/.last-profile`  
- `.ai-code-index/.last-indexed-at`  
- `.ai-code-index/.reindex.lock/`  

## Agent search order

1. `search.sh --profile <scope> "<intent>"`  
2. `symbols.sh <Name> [--lang …] [--kind …]`  
3. `files.sh --profile <scope> [--ext …] "<hint>"`  
4. `struct-search.sh <lang> '<pattern>'` for shape rewrites  
5. `stats.sh --profile <scope>` before broad audits  
6. Fall back to `rg` only for unindexed/generated trees  

Prefer this index over whole-repo `rg` when discovery is the goal.
