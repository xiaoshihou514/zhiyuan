# 三层记忆架构（RocksDB）

## 概述

RocksDB 3 个列族（Column Family）存储于 `~/.cache/zhiyuan/<query_hash>/`，通过 `MemoryManager` 统一管理。

```
MemoryManager
├── working: WorkingMemory    # 当前研究会话的工作数据
├── episodic: EpisodicMemory  # 按迭代编号存储的原始发现
└── semantic: SemanticMemory  # 按主题关键词检索的历史发现
```

## 工作记忆 (Working Memory)

```rust
pub struct WorkingMemory {
    db: DB,
    cf: String,  // "working"
}
```

简单的 key-value 存储，供 Orchestrator 读写当前会话状态：

| key | value |
|-----|-------|
| `"plan"` | 序列化的 `ResearchPlan` JSON |
| `"citation_graph"` | 序列化的 `CitationGraph` JSON |
| `"report"` | 序列化的 `ResearchReport` JSON |
| `"iteration:{n}:quality"` | 序列化的 `QualityScore` JSON |

方法：`set(key, value)` / `get(key)` / `clear()`

## 情节记忆 (Episodic Memory)

```rust
pub struct EpisodicMemory {
    db: DB,
    cf: String,  // "episodic"
}
```

按研究 ID + 迭代编号存储处理的发现（Finding）。

**key 格式**：`{research_id}:iteration:{n}:{finding_id}`

- `research_id`：字符串标识，运行时固定为 `"current"`
- `iteration`：迭代编号（1-based）
- `finding_id`：UUID

方法：
- `store_iteration(research_id, iteration, finding)` — 写入单条发现
- `get_iteration_findings(research_id, iteration)` — 通过 prefix seek 读取某轮所有发现

## 语义记忆 (Semantic Memory)

```rust
pub struct SemanticMemory {
    db: DB,
    cf: String,  // "semantic"
}
```

跨研究的知识复用。存储实体和发现，支持按关键词检索。

**key 格式**：

| key | value |
|-----|-------|
| `"entity:{name}"` | 实体描述 JSON |
| `"finding:{topic}:{finding_id}"` | 序列化的 `Finding` JSON |

**`find_relevant_findings(query)`** — 基于关键词匹配的检索：
1. 遍历所有 `finding:*` key
2. 对每个 finding 的 content 做关键词匹配（query 中的词与 content 小写匹配）
3. 按匹配数排序，返回 top-20

在 `research()` 开始时调用，将匹配的历史 finding 作为初始上下文注入（`iteration: 0`）。

## 引用图 (CitationGraph)

**注意**：`CitationGraph` 是一个内存数据结构，存储在 `IterationState` 中，**不直接存储在 RocksDB 中**（通过 `working.set("citation_graph", ...)` 序列化暂存）。

```rust
pub struct CitationGraph {
    pub claims: Vec<Claim>,      // {id, text, confidence}
    pub sources: Vec<SourceNode>, // {id, url, title, reliability}
    pub edges: Vec<CitationEdge>, // Supports(claim_id, source_id) | Contradicts(claim_id, source_id)
}
```

由 `VerifierAgent::verify_claims()` 构建，不依赖 petgraph（使用 Vec 存储）。

## 数据生命周期

| 阶段 | 操作 |
|------|------|
| 研究启动 | 打开 RocksDB，从 semantic memory 检索相关历史发现 |
| 每轮迭代 | working：更新 plan/citation_graph/report；episodic：写入新 finding |
| 研究结束 | 将所有 finding 存储到 semantic memory（按子任务主题归类） |
| 会话关闭 | RocksDB 实例析构，数据持久化到磁盘 |
