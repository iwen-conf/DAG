# 03 · 服务 API

> D-18：无认证。请求头 `X-Sunmao-Actor: human | agent:<id>` 自报身份（缺省拒绝——不是鉴权，是审计与租约归属需要，D-17/D-05）。
> D-19：**多项目**——除 `/v1/projects*` 外，所有端点挂在 `/v1/projects/{pid}/` 前缀下（下文表格省略前缀）。
> 暴露面分层是 [05-角色与协作](../00-需求分析/05-角色与协作.md) 权限矩阵在无认证下的实现（D-18）：**范围类端点不写进 Agent 的工具说明**，且 server 对 `actor` 前缀为 `agent:` 的范围操作请求返回 403（软校验，`manual: true` 的落地）。

## 3.0 项目管理（D-19，无前缀）

| 端点 | 语义 | 使用者 |
|------|------|--------|
| `POST /v1/projects` | 注册项目 `{name, repo_path}`；repo_path 已存在则幂等返回既有项目 | `sunmao init` |
| `GET  /v1/projects` | 项目列表（含各项目聚合进度） | CLI TUI |
| `GET  /v1/projects/lookup?repo_path=` | 按仓库路径反查（CLI 目录绑定解析第 3 级） | `sunmao` 各命令 |
| `GET  /v1/projects/{pid}` | 项目详情 | CLI TUI |

## 3.1 端点总表（均在 `/v1/projects/{pid}/` 下）

### Worker 面（写进 Worker Agent 的工具说明）

| 端点 | 语义 | 决策 |
|------|------|------|
| `POST /tasks/claim-next` | 原子认领下一个匹配任务；无任务时返回 204 + 提示（可 Expand 的 Package 列表） | D-05、FR-05.6 |
| `POST /tasks/{id}/heartbeat` | 续租；回带 `lease_token` | D-05 |
| `POST /tasks/{id}/handover-review` | 接力任务：上报现场审查结论（接手后、开工前必调） | D-05 ③ |
| `POST /tasks/{id}/submit` | 声明完工，触发 diff 核验 + Validator；回带 `lease_token` | D-10、FR-06.2 |
| `POST /tasks/{id}/fail` | 主动报告无法完成（原因入 attempt） | FR-06.3 |
| `GET  /tasks/{id}` | 任务详情（spec、inputs 的 artifact 引用、交接上下文） | FR-04 |
| `GET  /artifacts/{id}` | 读产物元数据（内容经 commit hash 从项目树读） | FR-07.3 |

### Planner 面（写进 Planner Agent 的工具说明）

| 端点 | 语义 | 决策 |
|------|------|------|
| `GET  /graph?root={id}&depth=N` | 读子图（含派生态） | FR-02.4 |
| `POST /graph/publish` | 提交图变更（新增/修改节点与边），服务端跑编译校验，过则原子发布为新 graph_version | **D-01 全自动的唯一入口** |
| `POST /contracts/{id}/publish` | 发布 Contract 版本，声明 bump 级别；major → 挂起待人批 | D-08 |
| `GET  /replan-context?task={id}` | 失败任务的重规划上下文（attempts、受影响子图） | D-07 |

### 人类面（仅 sunmao 使用，不进任何 Agent 工具说明）

| 端点 | 语义 | 决策 |
|------|------|------|
| `POST /packages/{id}/scope` | `{action: pause\|close\|reopen\|archive, reason, until?, permanent?, force?}` | FR-09、D-03、D-04 |
| `POST /contracts/{id}/approve-major` | 放行破坏性 Contract → 触发影响标记 | D-08 |
| `POST /tasks/{id}/cancel` | 取消单任务（终租约；`--force` 时丢弃在制品） | D-04 |
| `GET  /events?after_seq=N&node={id}` | 审计时间线分页拉取 | FR-13 |
| `GET  /events/stream` (SSE) | 实时事件流（源自 NOTIFY，按项目过滤） | D-17、FR-12 |
| `POST /admin/rebuild-projection` | 事件流重放重建投影 | A-03 |

## 3.2 关键端点契约

### claim-next

```jsonc
// POST /v1/projects/{pid}/tasks/claim-next
// 请求
{ "capabilities": ["rust", "database"], "lease_ttl_secs": 900 }

// 200 —— 普通任务
{
  "task": {
    "id": "nd_01J...", "title": "实现 LoginService",
    "spec": { "goal": "...", "acceptance": ["..."] },
    "inputs": [ { "artifact_id": "ar_01J...", "version": "1.2.0",
                  "paths": ["contracts/identity.yaml"], "commit": "a1b2c3" } ],
    "write_scope": ["server/identity/application/"],
    "validators": ["cargo-check", "scope-diff"],
    "attempt_seq": 1
  },
  "lease": { "token": "8c6f...", "expires_at": "2026-07-16T09:30:00Z" }
}

// 200 —— 接力任务（attempt_seq > 1）：多一个 handover 块（D-05 ②④）
{
  "task": { "...": "...", "attempt_seq": 2 },
  "lease": { "...": "..." },
  "handover": {
    "previous_attempts": [ {
        "seq_no": 1, "owner": "agent:w1", "outcome": "lease_expired",
        "failure": null, "handover_report": null
    } ],
    "work_in_progress": {              // 写范围内未 commit 的变更（git status 扫描）
      "modified": ["server/identity/application/login.rs"],
      "untracked": ["server/identity/application/dto.rs"]
    },
    "progress_snapshot": {             // D-05 ④ 进度同步
      "graph_version": 42,
      "upstream_artifacts": [ { "artifact_id": "ar_...", "version": "1.2.0" } ]
    },
    "instruction": "你接手的是一个中断任务。必须先审查 work_in_progress 中列出的在制品，判断可复用或需清理重做，并调用 handover-review 如实上报结论后再开工。不得隐瞒现场情况。"
  }
}

// 204 —— 无任务；body 提示下钻方向（FR-05.6「无任务则下钻」）
{ "hint": { "expandable_packages": ["nd_..（server.payment, plan_state=draft）"] } }
```

### handover-review（D-05 ③ 如实上报）

```jsonc
// POST /v1/projects/{pid}/tasks/{id}/handover-review   （lease_token 必带）
{
  "lease_token": "8c6f...",
  "wip_assessment": "login.rs 已完成 70%，结构可复用；dto.rs 为空文件",
  "decision": "reuse",                  // reuse | discard_and_redo | partial
  "discarded_paths": [],
  "concerns": "上一手未处理 token 过期分支"
}
// → 200；写 attempt.handover + event task.handover_reported
// 服务端约束：接力任务未调本端点前，submit 返回 409 HANDOVER_REVIEW_REQUIRED
```

### submit（完工链，D-10/D-11/D-15）

```jsonc
// POST /v1/projects/{pid}/tasks/{id}/submit
{ "lease_token": "8c6f...", "note": "实现完成，本地 cargo check 通过" }

// 服务端同步执行（详见 04 §流程一）：
//   git diff 越界检查 → scope 内变更归集 → 注册 Validator 依次跑
// 200
{ "verdict": "done", "artifact": { "id": "ar_...", "commit": "d4e5f6" } }
// 422 —— 核验/验证失败（attempt 关闭，按 D-06 决定重开或 Failed）
{ "verdict": "failed",
  "failures": [ { "validator": "scope-diff",
      "report": "越界写入: docs/readme.md 不在声明范围 server/identity/application/" } ],
  "next": "reopened"   // reopened | failed_final
}
```

### graph/publish（D-01 全自动门）

```jsonc
// POST /v1/projects/{pid}/graph/publish
{
  "base_version": 42,                  // 乐观锁：不等于当前版本 → 409（D-12 单写者）
  "summary": "展开 server.identity 为 contract/application/infrastructure",
  "upsert_nodes": [ { "id": "nd_new1", "parent_id": "nd_identity",
                      "kind": "task", "title": "...", "spec": {},
                      "write_scope": ["server/identity/contract/"],
                      "required_caps": ["rust"], "validators": ["cargo-check"] } ],
  "add_edges": [ { "from": "nd_new1", "to": "nd_new2" } ],
  "remove_nodes": []                   // 仅允许移除无 attempt 历史的节点
}
// 200 { "version": 43, "ready_now": ["nd_new1"] }
// 422 —— 编译校验拒绝（FR-03.4：给出具体节点与路径）
{ "violations": [
    { "rule": "cycle", "path": ["nd_a","nd_b","nd_a"] },
    { "rule": "write_conflict", "nodes": ["nd_x","nd_y"], "scope": "server/identity/" },
    { "rule": "dangling_dep", "edge": {"from":"nd_ghost","to":"nd_new2"} } ] }
```

## 3.3 错误码约定

| HTTP | code | 场景 |
|------|------|------|
| 401 | `MISSING_ACTOR` | 无 `X-Sunmao-Actor` 头 |
| 403 | `HUMAN_ONLY` | agent 调范围/审批端点（05 权限矩阵） |
| 409 | `LEASE_LOST` | lease_token 不匹配或已过期（fencing，含幽灵提交拒收） |
| 409 | `STALE_GRAPH_VERSION` | publish 乐观锁失败 |
| 409 | `HANDOVER_REVIEW_REQUIRED` | 接力任务未审查先提交 |
| 422 | `VALIDATION_FAILED` / `GRAPH_INVALID` | submit 验证失败 / publish 校验失败 |

所有错误 body：`{ code, message, details }`，details 必须包含可定位信息（14.1 Fail Fast 2——节点、路径、根因，不吞错）。

## 3.4 SSE

```text
GET /v1/projects/{pid}/events/stream?after_seq=1024
event: task.done
data: {"seq":1025,"node_id":"nd_...","payload":{...}}
```

- 断线重连用 `after_seq` 补拉（event.seq 全序）；
- Worker 可选订阅代替轮询 claim-next（拿到 `graph.published`/`task.done` 再去 claim），v1 轮询兜底。
