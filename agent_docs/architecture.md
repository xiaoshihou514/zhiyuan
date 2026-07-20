# 致远 (Zhiyuan) 深度研究框架 — 架构设计

## 项目概述

致远是一个基于 Rust 生态的深度研究（Deep Research）框架，实现迭代式自适应研究流程，通过 LLM 驱动的研究智能体组合，完成从问题理解到报告生成的完整闭环。不依赖第三方 Agent 框架（如 rig），所有智能体直接通过 `LlmClient` trait 调用 LLM。

## 三层架构

```
┌───────────────────────────────────────────────────┐
│  编排层 (Orchestration Layer)                      │
│  ResearchOrchestrator | QualityEvaluator           │
│  迭代控制 | 任务并发调度 | 渐进式报告构建          │
├───────────────────────────────────────────────────┤
│  智能体层 (Agent Layer) — 7 个 LLM 驱动 Agent      │
│  Planner | QueryPlanner | Searcher                │
│  ExtractorAgent | Synthesizer | Verifier | Writer  │
│  (全部通过 LlmClient::prompt() 直接调用 LLM)       │
├───────────────────────────────────────────────────┤
│  基础层 (Foundation Layer)                         │
│  EnginePool (SearXNG) | WebExtractor | MemoryMgr   │
│  LlmClient | Retry/Timeout | JSON 规范化           │
└───────────────────────────────────────────────────┘
```

## Crate 依赖关系（DAG）

```
                  ┌──────────────┐
                  │  zhiyuan-core │ ← 核心类型、trait、错误枚举
                  └──────┬───────┘
         ┌───────────────┼───────────────────┐
         ▼               ▼                   ▼
  zhiyuan-search  zhiyuan-extract    zhiyuan-memory
  (SearXNG 引擎)   (网页/PDF 提取)    (RocksDB 3 列族)
         │               │                   │
         └───────────────┼───────────────────┘
                         ▼
                  zhiyuan-robust
              (重试/超时/JSON 验证)
                         │
                         ▼
                  zhiyuan-agents
          (7 个 Agent，调用 LlmClient)
                         │
                         ▼
              zhiyuan-orchestrator
           (编排 + 质量评估 + 迭代控制)
```

| Crate | 职责 | 关键依赖 |
|-------|------|---------|
| `zhiyuan-core` | 核心类型 (`ResearchQuery`, `Finding`, `CitationGraph`, `ArgumentSkeleton`, `EpistemicStatus` 等)、`LlmClient` trait、`ResearchConfig`、错误枚举 | serde, thiserror, uuid |
| `zhiyuan-embedding` | `EmbeddingProvider` trait, `LocalEmbedder` (bge-large-zh), `VectorIndex` (HNSW), `NoopEmbedder` 回退桩 | fastembed, instant-distance |
| `zhiyuan-search` | `EnginePool` + `SearXngEngine`，搜索去重+相关度过滤 | reqwest, serde |
| `zhiyuan-extract` | `WebExtractor`：dom_smoothie(Readability) → Markdown，PDF(pdf_oxide)，URL 缓存 | dom_smoothie, pdf_oxide |
| `zhiyuan-memory` | `MemoryManager`：RocksDB 三列族 (working/episodic/semantic) | rocksdb, serde |
| `zhiyuan-robust` | `with_retry()` 指数退避、`with_timeout()`、`MessageNormalizer` JSON 校验 | tokio, backoff |
| `zhiyuan-agents` | 7 个 Agent（`PlannerAgent`, `QueryPlannerAgent`, `SearcherAgent`, `ExtractorAgent`, `SynthesizerAgent`, `VerifierAgent`, `WriterAgent`） | zhiyan-core, zhiyuan-search, zhiyuan-extract |
| `zhiyuan-orchestrator` | `ResearchOrchestrator` 主循环 + `QualityEvaluator` 四维评分 + `ArgumentBuilder` | zhiyuan-agents, zhiyuan-memory |

## 二进制入口 (`src/main.rs`)

- CLI 解析 → 生成 session hash → 加载配置 → 创建 EnginePool + LLM Client
- **澄清阶段**：PlannerAgent 生成 2-4 个澄清问题，TUI 交互收集用户反馈
- **研究阶段**：ResearchOrchestrator::research() 迭代运行
- **PDF 生成**：使用 `typst` + `typst-pdf` crate 编译 Typst 源码（内置 CJK 字体加载 + LLM 修复错误，最多 5 轮）

## 配置系统 (`zhiyuan.toml`)

查找顺序：`~/.config/zhiyuan.toml` → `./zhiyuan.toml`。各节：

| 节 | 键 | 默认值 | 说明 |
|----|----|--------|------|
| `[search]` | `max_results` | 10 | 每引擎结果数 |
| | `searxng_url` | `http://localhost:8888` | SearXNG 实例地址 |
| | `blocked_domains` | `[]` | 屏蔽域名 |
| `[llm]` | `api_key` | `""` | OpenAI 兼容 API key |
| | `base_url` | `https://api.deepseek.com/v1` | API 端点 |
| | `main_model` | `deepseek-v4-flash` | 模型名 |
| `[research]` | `concurrency` | 4 | 并发数 |
| | `max_iterations` | 4 | 最大迭代轮次 |
| | `long_report` | false | 长报告模式 |
| | `cross_validate` | false | 交叉验证 |
| `[pdf]` | `font_paths` | `[]` | Typst 字体路径 |
| `[embedding]` | `enabled` | `true` | 是否启用向量检索 |
| | `model` | `"bge-large-zh"` | embedding 模型名 |

## 目录结构

| 路径 | 用途 |
|------|------|
| `~/.config/zhiyuan.toml` 或 `./zhiyuan.toml` | 应用配置 |
| `.env` | 可选 dotenv |
| `~/.cache/zhiyuan/<query_hash>/` | RocksDB 记忆存储 |
| `~/.local/share/zhiyuan/<hash>.log` | tracing 日志 |
| `template/lib.typ` | Typst 模板 |
| `template/icon.svg` | 水墨风封面图标 |
