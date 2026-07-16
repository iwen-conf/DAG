# sunmao（榫卯）

多项目 DAG 任务编排服务：**Contract First**、事件+投影、Git 产物、无 merge 拼装。

交付物（D-19/D-20）：

| 二进制 | 作用 |
|--------|------|
| `sunmao-server` | 唯一常驻服务：图 / 任务 / 租约 / 事件 / 产物 |
| `sunmao` | 人用 CLI（TUI 配置与项目列表 + 目录绑定） |

Planner / Worker 是**外部** Agent，只调 HTTP。

## Workspace

```
crates/
  sunmao-core/    # 纯领域（零 IO）
  sunmao-store/   # PostgreSQL + git
  sunmao-server/  # HTTP / SSE / reaper
  sunmao-cli/     # sunmao
docs/
scripts/
  pg-tunnel.sh    # SSH 转发远程 PG（禁止本机装库）
  e2e-smoke.sh    # 冒烟
```

## 开发：远程 Postgres + SSH tunnel

**不要在本机安装 PostgreSQL。**

```bash
# 终端 A：隧道（按环境改 host/端口/账号）
./scripts/pg-tunnel.sh
# 或: SSH_HOST=user@host REMOTE_PG=15432 LOCAL_PG=15432 ./scripts/pg-tunnel.sh

# 终端 B
export DATABASE_URL='postgres://USER:PASS@127.0.0.1:15432/sunmao'
cargo run -p sunmao-server -- --db "$DATABASE_URL" --listen 127.0.0.1:7420
```

```bash
# 纯逻辑
cargo test -p sunmao-core

# 集成（需隧道 + DATABASE_URL）
cargo test -p sunmao-store --test integration_flow -- --test-threads=1

# 冒烟
./scripts/e2e-smoke.sh
```

## CLI

```bash
sunmao config --base-url http://127.0.0.1:7420   # 或交互 TUI
sunmao projects                                  # TUI 列表
cd /path/to/repo && sunmao init
sunmao tree
sunmao task show nd_xxx
sunmao scope close <package_id> --reason "..."
sunmao approve-major ar_xxx
sunmao admin verify
```

请求头：`X-Sunmao-Actor: human | agent:<id>`。

## 里程碑实现状态

| 里程碑 | 状态 |
|--------|------|
| M0 骨架与数据层 | ✅ 完成 |
| M1 图引擎与编译校验 | ✅ 完成 |
| M2 调度与租约 | ✅ 完成 |
| M3 完工链与接力 | ✅ 完成 |
| M4 Contract / 事件 / CLI | ✅ 完成 |
| M5 毕业考 | ✅ 完成 |

细节与已知例外：`docs/01-架构设计/05-实施计划.md` 文首；架构索引：`docs/01-架构设计/README.md`。
