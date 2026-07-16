# Repository Map

## Overview

**DAG** — AI Project Operating System / AI Build System (Rust, planned).

| Area | Path | Status |
| --- | --- | --- |
| Requirements | `docs/00-需求分析/` | Active |
| Index tooling | `.ai-code-index/` | Active |
| Rust workspace | `src/`, `crates/`, `Cargo.toml` | Reserved (not created yet) |
| Tests | `tests/`, `benches/` | Reserved |

## Requirements map (`docs/00-需求分析`)

| Doc | Role |
| --- | --- |
| `README.md` | Index & reading order |
| `00-对话演进与结论.md` | 11-turn consensus |
| `01`–`07` | Runtime: vision, pipeline, nodes, states, roles, schedule, protocol |
| `08`–`12` | Foundations: boundaries, parallel rules, Contract First, Task IR, product fit |
| `ChatGPT-拆分任务边界方法.md` | Full conversation export (source of truth) |
| `99-对话原文摘录.md` | Per-turn navigation |

## Planned runtime packages (from requirements)

Not in tree yet; used for future profile path design:

```text
Task Service / DAG Server
Planner + Replanner
Scheduler (Pull model)
Artifact Store
Validator
Worker Agents (capability-based Claim)
```

Core objects: **Artifact + DAG + Contract**.

## Profile → paths

| Profile | Roots |
| --- | --- |
| `code` | rust paths if exist + `docs/00-需求分析` + meta |
| `rust` | `src`, `crates`, `tests`, `benches`, `examples`, Cargo files |
| `docs` | `docs/00-需求分析` |
| `docs-full` | `docs` |
| `meta` | root configs + `.ai-code-index` scripts |
| `all` | rust + full docs + meta |
| `ref` | `reference`, `vendor` (optional) |

## Ignore highlights

- `target/` (Rust build)  
- `.ai-code-index/index/`, `tags`, freshness markers  
- `node_modules`, `vendor`, `dist`, `.git`  
