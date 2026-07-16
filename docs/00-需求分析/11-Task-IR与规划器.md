# 11 · Task IR 与规划器

> 来源：完整对话第 4–5、8 轮  
> 用途：说明 DAG 如何生成、校验、演化；**Planner / IR 是价值最高的模块之一**。

## 11.1 为什么需要「程序」生成 DAG

DAG 数据结构简单（拓扑排序、环检测几十年前就成熟）。  
真正难、真正值钱的是：

> 把模糊需求**编译**成可执行的 Task DAG。

该模块可称为 **Task Planner（任务规划器）** 或 **DAG Compiler（任务图编译器）**。  
它不写业务代码，只产出图。

```text
用户需求
    │
    ▼
Requirement Analyzer
    │
    ▼
Task Planner
    │
    ▼
Task DAG
    │
 ┌──┼────┬─────┐
Agent1… AgentN
    │
 Artifact Merger
```

## 11.2 编译器类比

```text
传统：源码 → Lexer → Parser → AST → IR → Optimizer → Machine Code

AI 工程：
需求 → Requirement AST → Task IR → Task DAG → Scheduler → Agent → Artifacts
```

最重要的往往不是 Agent，而是 **Task IR（中间表示）**。

AI 不应直接吃自然语言需求去写全站代码，应先变成统一 IR。

## 11.3 Task IR 示例（草案）

```yaml
task:
  id: task-001
  type: design-db
  output:
    - schema.sql
  depends: []

task:
  id: task-002
  type: generate-api
  input:
    - schema.sql
  output:
    - openapi.yaml
  depends:
    - task-001

task:
  id: task-003
  type: generate-go
  input:
    - openapi.yaml
  output:
    - go/
  depends:
    - task-002
```

Scheduler **不必懂「小说网站」**，只看 DAG / IR。

## 11.4 三种生成路线

### 路线 A：规则生成（Deterministic）

```text
有 schema → 一定先 API → 再 SDK → 再前端
```

| 优点 | 缺点 |
|------|------|
| 稳定、可预测 | 不灵活 |

### 路线 B：纯 AI 规划

```text
Requirement → Planner LLM → Task DAG → Scheduler
```

灵活，但易出错（环、写冲突、边界错）。

### 路线 C：混合模式（推荐）

```text
需求
  ▼
LLM Planner → Task Draft
  ▼
Rule Checker
  ▼
Dependency Analyzer
  ▼
Cycle Checker
  ▼
Conflict Checker（写冲突）
  ▼
Scheduler
```

原则：

> **AI 负责创造；程序负责验证。**

| AI 输出 | 程序动作 |
|---------|----------|
| `A→B→C→A` | 拒绝：Cycle |
| 两 Task 同写 `schema.sql` | 拒绝：Write Conflict |

类比 Make / Cargo / Bazel / Ninja：都是 **构建 DAG**；AI 工程只是把节点换成 Requirement / Design / API / Go / Test / Deploy。

## 11.5 系统五件套（Build System 视角）

| 组件 | 职责 |
|------|------|
| **Planner** | 需求 → Task DAG |
| **Scheduler** | 按依赖并行调度 |
| **Artifact Store** | 保存 SQL / OpenAPI / 代码 / 文档等 |
| **Validator** | 检查产物是否满足契约 |
| **Replanner** | 失败时只重规划受影响子图，不推倒全项目 |

→ **AI Build System**，不是「很会聊天的 Agent」。

## 11.6 可演化 DAG（非静态完整图）

用户直觉：多会话直到「数据结构全部完成」。

修正：

- 初始 DAG 由 Planner 根据需求生成；
- 节点完成后 Validator 检查；
- 验证失败 / 需求变化 / 新依赖 → **只改受影响子图**。

类比增量编译：

> 不是一张静态 DAG，而是一张**可演化的 DAG**。

## 11.7 层级 DAG 与按需展开（大包小）

规划能力本身也是分层的：AI 不擅长一次规划 500 个 Task，擅长：

```text
Project → Server/Client
Server  → Identity/Novel/User
Identity → Contract/Application/Infrastructure
…
```

**规划也是递归的。** Scheduler 可 Lazy Loading：只展开当前需要的子树。

节点统一结构示例：

```yaml
id: server.identity
type: package
status: pending
children:
  - contract
  - application
  - infrastructure
depends_on:
  - shared.openapi
artifacts:
  - identity/*
```

`children: []` → 叶子 → 交给 Agent。

节点可「自展开」：用户或 Worker 点到非叶子 → Planner 实时生成子节点。

## 11.8 与 LangGraph 类系统的差别

| LangGraph 类 | 本系统 DAG |
|--------------|------------|
| 推理流程：LLM→Tool→LLM | 软件结构：Package→Module→Feature→Task→Artifact |
| 关注对话/工具链 | 关注工程分解与产物 |

## 11.9 规划难点（Planner 必须懂架构）

| 难点 | 说明 |
|------|------|
| Task 从哪来 | 自然语言 → 合理任务树，非算法题 |
| 边界 | Login 属 User 还是 Identity？JWT 属 Login 还是 Gateway？ |
| 上下文污染 | Permission 读 User，User 又依赖 Permission → 环，必须拆 |
| Merge | 两 Agent 改同文件如何合并 |
| 验证 | Schema / OpenAPI / 代码不一致时谁负责 → Validator |

详见 [12-难点与产品定位.md](./12-难点与产品定位.md)。
