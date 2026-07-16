# 10 · Contract First 与架构并行

> 来源：完整对话第 6–7 轮  
> 用途：说明如何用架构分层与契约冻结，让「业务 ∥ 基础设施 ∥ 多端」真正并行。

## 10.1 从任务拆分到架构拆分

用户侧直觉：Controller / Business / Infrastructure 分层，先做只有外向依赖的基础设施，业务只调接口。

对话确认方向正确（依赖倒置 + Ports & Adapters），并强调：

> **业务层不应依赖「基础设施层」，而应依赖业务定义的 Port（契约）。**

```text
Business
├── UserService
├── UserRepository (interface)   ← Port
└── Storage (interface)          ← Port

Infrastructure
├── MysqlUserRepository
├── S3Storage
└── RedisCache

依赖方向：Infrastructure ──实现──► Business（定义接口）
```

Port 一旦冻结，Business Agent 与 Infrastructure Agent 可真正独立，最后按接口拼接。

## 10.2 四阶段并行流水线

### 阶段 1：Contract（契约）— 先冻结

只生成 Interface / 契约，不写实现、不写业务：

```text
UserRepository / NovelRepository / Storage / Cache / Logger / Config / Router
```

```go
type UserRepository interface {
    FindByID(...)
    Save(...)
}
```

接口定了 = 项目一部分已「冻结」。

### 阶段 2：基础设施并行

多 Agent 互不感知，只认 Interface：

```text
Storage | Cache | Logger | HTTP | Database | Config
```

彼此无依赖 → 高度并行。

### 阶段 3：业务层

```text
User | Novel | Comment | Order
```

只调用 Port（如 `UserRepository`），不知 MySQL / Redis / Mongo。

### 阶段 4：Controller / API

```text
User API | Novel API | Comment API | Order API
```

调用 Application/Service，再并行。

### 架构天然导出 DAG

```text
          Contract
              │
      ┌───────┴────────┐
 Infrastructure    Business
      └────────┬───────┘
          Controller
               │
              UI
```

**不是人为硬画图，而是架构决定了 DAG。**

## 10.3 Contract First + DAG First

未来开发不是单纯 Code First 或 API First，而是：

> **Contract First + DAG First**

```text
contract/
  user.yaml
  novel.yaml
  order.yaml
planner/
graph.json
```

```text
Planner → 生成 DAG → Scheduler → N 个 Agent → Merge
```

Worker 只知：

> 我是 Logger Agent；输入 LoggerContract；输出 logger.go。完成即退出。

## 10.4 不要按项目目录划 DAG，按稳定性划

用户划分：

```text
Project
├── Server
└── Client
    ├── User / Admin / Writer / Customer Service
```

这是**项目结构**，不是依赖图。

DAG 应按 **依赖稳定性（Stability）**：

```text
Requirement
    ▼
Domain Model
    ▼
API Contract (OpenAPI)   ← 系统最稳定枢纽之一
    ├──────────────┐
 Server          Client SDK
    ▼              ▼
 Business     User/Admin/Writer/…
```

OpenAPI 冻结后：

- 管理端 / 用户端 / 作者端 / 客服端可全部并行；
- 甚至不知 Server 是否写完；
- **Mock 足够**。

## 10.5 服务端再拆：Context × 层

```text
Server
├── Identity / User / Novel / Chapter / Comment
├── Search / Payment / Statistics / Notification
```

每个 Context 内再拆：

```text
Identity
├── Domain
├── Repository Interface   ← Contract
├── Application
├── API
├── Repository Impl
└── Test
```

先冻结 Port（`UserRepository` / `TokenProvider` / `PasswordHasher`），再并行 Application ∥ Infrastructure ∥ Controller。

不同 Context（Identity / Novel / Comment / Payment）之间几乎无共享写 → 多 Agent 并行。

## 10.6 建议的架构阶段链

```text
Requirement
    ▼
Architecture      ← 不应由 Implementation 反推
    ▼
Contract
    ▼
Implementation
    ▼
Verification
```

Planner 先输出 Context 列表，再输出每个 Context 的 Port / DTO / Events / API，**Contract 固定后**才 Implementation。

## 10.7 四种节点角色（工程视角）

对话建议抽象四类节点（与 Package/Task 正交，可作标签）：

| 类型 | 示例 | 特点 |
|------|------|------|
| **Specification** | Requirement / Architecture / Domain / OpenAPI / Schema | 定义规则 |
| **Generator** | Go / Vue / SQL / Docs | 只读 Specification，不改规范 |
| **Validator** | Compile / Lint / Unit Test / API Check | 不产生业务代码，只判断 |
| **Integration** | Merge / Deploy / Release / Package | 收口 |

## 10.8 核心对象

面向软件工程的系统应设计：

```text
Artifact + DAG + Contract
```

而不是：

```text
Conversation + Memory + Agent
```

因为最终交付的是产物，Agent 只是生成产物的执行器。
