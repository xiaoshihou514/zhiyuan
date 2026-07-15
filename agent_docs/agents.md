# 智能体设计 (基于 rig)

## 六类智能体

| 智能体      | 职责                            | 模型层级  | rig 组件                                 |
| ----------- | ------------------------------- | --------- | ---------------------------------------- |
| Planner     | 分析查询、生成研究计划/子任务图 | Reasoning | rig::Agent + Tool                        |
| Searcher    | 构造搜索查询、调用搜索引擎      | Fast      | rig::Agent + Tool(SearchEngine)          |
| Extractor   | 抓取网页、提取结构化信息        | Fast      | rig::Agent + Tool(HttpFetch)             |
| Synthesizer | 综合信息、更新知识库、生成摘要  | Main      | rig::Agent + Tool(Memory)                |
| Verifier    | 交叉验证关键声明、检测矛盾      | Reasoning | rig::Agent + Tool(CitationGraph)         |
| Writer      | 渐进式报告构建、引用标注        | Main      | rig::Agent + Tool(Memory, CitationGraph) |

## 模型层级路由

```rust
pub enum ModelTier {
    Reasoning,  // 复杂推理 (如 o3-mini, claude-sonnet)
    Main,       // 标准任务 (如 gpt-4o, claude-3.5)
    Fast,       // 轻量任务 (如 gpt-4o-mini, claude-haiku)
}

pub struct ModelRouter {
    providers: HashMap<ModelTier, rig::providers::Provider>,
    budget_tracker: CostBudget,
}
```

- 根据任务复杂度自动路由到合适模型
- 支持成本预算控制（超预算降级或终止）

## rig 集成方式

```rust
// 示例：Searcher Agent
let searcher = rig::agent::AgentBuilder::new(provider.clone())
    .preamble("你是一个搜索专家，根据研究目标构造有效的搜索查询。")
    .tool(SearchTool::new(engine_pool.clone()))
    .build();

// 示例：rig pipeline 线性工作流
let pipeline = rig::pipeline::new()
    .chain(planner_agent)
    .chain(searcher_agent)
    .chain(extractor_agent)
    .chain(synthesizer_agent)
    .chain(writer_agent);
```

## 工具定义

每个智能体通过 rig 的 Tool 系统暴露能力：

- `SearchTool` — 调用 `EnginePool.search()`
- `FetchTool` — 调用 `Fetcher.fetch_url()`
- `MemoryTool` — 读写三层记忆
- `CitationTool` — 操作引用图
- `EvaluateTool` — 触发质量评估
