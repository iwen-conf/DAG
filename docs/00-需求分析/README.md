# 00 · 需求分析

> **完整原文**：[ChatGPT-拆分任务边界方法.md](./ChatGPT-拆分任务边界方法.md)（11 轮全量导出）  
> 分享链接（仅后半段）：[chatgpt.com/share/…](https://chatgpt.com/share/6a58446a-30b8-83ec-a45b-4fdc72ed343e)  
> 整理日期：2026-07-16（按完整导出增补）

## 一句话定义

本项目不是「多 Agent 聊天框架」，而是 **AI Project Operating System / AI Build System**：

- **Contract** 定边界  
- **层级可演化 DAG** 定顺序与展开  
- **Pull + Claim** 调度自治 Worker  
- **Artifact** 替代长会话 Memory  
- **人** 只控范围（Close / Pause / Reopen）

产品隐喻：**AI 时代的 Cargo/Bazel** + GitHub Projects/Jira + K8s 调度思想。

## 文档索引

### 总览

| 文档 | 内容 |
|------|------|
| [00-对话演进与结论.md](./00-对话演进与结论.md) | 11 轮索引、抬升路径、不可妥协共识 |
| [01-系统愿景.md](./01-系统愿景.md) | 定位、隐喻、原则、成功场景 |
| [02-端到端流水线.md](./02-端到端流水线.md) | 需求→架构→DAG→Task→调度→产物→验证 |
| [03-节点模型.md](./03-节点模型.md) | 层级 DAG、Package/Task、Lazy Expand |
| [04-状态机.md](./04-状态机.md) | Package / Task 双状态机、Closed / Ignore |
| [05-角色与协作.md](./05-角色与协作.md) | Planner / Worker / Validator / 人机权限 |
| [06-调度与任务服务.md](./06-调度与任务服务.md) | Task Service、Pull、能力匹配 |
| [07-节点协议.md](./07-节点协议.md) | 节点字段与调度伪代码 |

### 方法论与根基（完整对话前半段增补）

| 文档 | 内容 |
|------|------|
| [08-边界拆分方法论.md](./08-边界拆分方法论.md) | 变化原因、七层法、五问法、Bounded Context |
| [09-小任务与并行条件.md](./09-小任务与并行条件.md) | 四条件、五并行条件、IO 模型、写权限 |
| [10-Contract-First与架构并行.md](./10-Contract-First与架构并行.md) | Port/DIP、四阶段并行、稳定性划分 |
| [11-Task-IR与规划器.md](./11-Task-IR与规划器.md) | Planner、IR、混合校验、可演化 DAG |
| [12-难点与产品定位.md](./12-难点与产品定位.md) | 五大难点、AI Cargo、非目标 |
| [13-详细功能需求清单.md](./13-详细功能需求清单.md) | 编号化、可验收的产品功能需求 |
| [14-非功能需求与待决策项.md](./14-非功能需求与待决策项.md) | 并发、一致性、可观测性、安全与产品决策 |
| [15-决策记录.md](./15-决策记录.md) | D-01~D-19 **全部落定**（封版 2026-07-16）：全自动规划、租约接力协议、semver 契约、PG+轻量 Git、无认证、非 Agent 多项目边界等 |

### 原文

| 文档 | 内容 |
|------|------|
| [ChatGPT-拆分任务边界方法.md](./ChatGPT-拆分任务边界方法.md) | 完整对话导出（唯一原文真源） |
| [99-对话原文摘录.md](./99-对话原文摘录.md) | 按轮次要点索引（指向完整导出） |

## 需求要点速览

1. **按变化原因拆边界**，不按表/巨型 Service。  
2. **小任务四条件**：一目标 / 一输入 / 一输出 / 一验证。  
3. **并行五条件**：输入固定、输出唯一、独占写、依赖明确、可独立验证。  
4. **Contract First**：Port 冻结后 Business ∥ Infra ∥ 多端。  
5. **Task IR + 混合 Planner**：AI 创造，程序查环/写冲突。  
6. **可演化层级 DAG**：大包小、按需展开，非一次静态完整图。  
7. **Agent = Developer**：Claim 循环；无任务则下钻 Expand。  
8. **Pull 非 Boss**；Scheduler 不懂业务。  
9. **Package = Workspace**；Close/Ignore 仅人触发。  
10. **核心对象**：Artifact + DAG + Contract。

## 待后续澄清

> 以下各项已在 [15-决策记录.md](./15-决策记录.md) 给出建议方案（待逐项确认）：

- Architecture / Contract 的正式 schema 与版本策略 → D-08
- 写冲突、Merge 与 Git 集成 → D-09 / D-10 / D-11 / D-15
- Validator 注册模型与 Replanner 细节 → D-06 / D-07
- 多 Planner 协作与冲突解决 → D-12
- Task IR 正式语法与确定性规则库（仍开放，属架构设计）
- Claim 租约、故障恢复、优先级和公平性策略 → D-05 / D-13
- Planner 草案的人工审批边界 → D-01
