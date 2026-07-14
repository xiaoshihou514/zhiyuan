# 三层记忆架构 (RocksDB)

## 设计

RocksDB 列族（Column Family）隔离三层记忆：

```
zhiyuan_memory/
├── working_memory/      # 当前迭代上下文（TTL 自动过期）
├── episodic_memory/     # 历史迭代发现（按迭代序）
└── semantic_memory/     # 跨研究结构化知识 + 向量索引
```

## 工作记忆 (Working Memory)

```rust
pub struct WorkingMemory {
    db: DB,
    cf: String,  // "working"
    /// 存储当前迭代的上下文
    /// key: "current_query" / "current_plan" / "iteration:N:findings"
}
```

- 每次迭代后清理旧条目、写入新条目
- 为 Orchestrator 提供当前完整上下文快照

## 情节记忆 (Episodic Memory)

```rust
pub struct EpisodicMemory {
    db: DB,
    cf: String,  // "episodic"
}

// key 格式: "research:{id}:iteration:{n}:summary"
// key 格式: "research:{id}:iteration:{n}:sources"
```

- 按 `research_id` + `iteration` 级别组织
- 支持跨迭代检索（回顾之前发现）
- 研究完成后可选择归档或清理

## 语义记忆 (Semantic Memory)

```rust
pub struct SemanticMemory {
    db: DB,
    cf: String,  // "semantic"
    vector_store: rig::vector_store::MemoryVectorStore,
}

// key 格式: "entity:{name}" → {type, relations, claims}
// key 格式: "claim:{id}" → {text, sources, confidence}
```

- 实体-关系-声明 结构化存储
- 向量索引用于语义相似度检索
- 跨研究复用知识，避免重复搜索

## 引用图 (Citation Graph)

```rust
pub struct CitationGraph {
    graph: petgraph::Graph<Node, Edge>,
    // Node: 声明(Claim) 或 来源(Source)
    // Edge: "支持" 或 "矛盾"
}
```

- 二部图：声明 ↔ 来源
- 多源支持 → 高置信度
- 矛盾检测 → 触发 Verifier 深入验证
