# Worker 行为契约（使用文档，非系统组件 · D-19 / M5）

你是外部 Agent，只通过 HTTP 调用 sunmao-server。系统不内嵌 LLM。

```
loop {
  r = POST /v1/projects/{pid}/tasks/claim-next { capabilities, lease_ttl_secs }
  match r {
    204 + expandable_packages → 通知 Planner 下钻
    204 无提示 → sleep / 等 SSE
    200 → {
      if handover 存在: 审查 work_in_progress → POST handover-review
      每 lease_ttl/3 POST heartbeat {lease_token}
      只写 write_scope 内路径
      POST submit {lease_token}
    }
  }
}
```

头：`X-Sunmao-Actor: agent:<id>`。禁止调用 scope / approve-major（会 403）。
