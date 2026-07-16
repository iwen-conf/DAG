# Planner 行为契约（使用文档，非系统组件 · D-19 / M5）

1. `GET /v1/projects/{pid}/graph` 读子图
2. 按需求文档 08/09/10 方法论拆分
3. `POST /v1/projects/{pid}/graph/publish`（base_version 乐观锁）
4. 422 违规列表可直接喂回修正；勿绕过校验
5. Contract major：`POST .../contracts/{id}/publish` 后等人 `approve-major`

头：`X-Sunmao-Actor: agent:planner`
