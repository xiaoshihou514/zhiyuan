# 致远 (Zhiyuan) 深度研究框架 — 架构设计

## 项目概述

致远是一个基于 Rust 生态的深度研究（Deep Research）框架，实现质量驱动的自适应迭代研究流程，支持多种搜索引擎和 LLM 后端。

## 三层架构

```
┌─────────────────────────────────────────────────┐
│  编排层 (Orchestration Layer)                     │
│  Research Orchestrator | Quality Evaluator       │
│  Cost Controller | Progress Tracker              │
├─────────────────────────────────────────────────┤
│  智能体层 (Agent Layer)                           │
│  Planner | Searcher | Extractor                  │
│  Synthesizer | Verifier | Writer                 │
│  (基于 rig Agent + Tool 系统)                     │
├─────────────────────────────────────────────────┤
│  基础层 (Foundation Layer)                        │
│  Model Router | Tool Registry                    │
│  Memory Manager (RocksDB) | Message Normalizer   │
│  Retry & Fault Isolation                         │
└─────────────────────────────────────────────────┘
```

## 六阶段流程

```
用户查询
    ↓
┌──────────────────────┐
│ 阶段一：意图理解与规划  │  Planner Agent → 研究计划 + 子任务图
│  (可选澄清)           │  Quality Evaluator → 初始质量评分
└──────────┬───────────┘
           ↓
┌──────────────────────┐
│ 阶段二：信息检索       │  Searcher Agent → 三引擎并行/故障切换
│ 阶段三：内容提取       │  Extractor Agent → 针对性信息提取
│ 阶段四：信息综合       │  Synthesizer Agent → 三层记忆更新
│ 阶段五：迭代深化       │  Verifier → 交叉验证
│  (质量驱动循环)        │  Quality Evaluator → 评分决策
│                      │  评分 < 阈值 → 继续迭代
│                      │  评分 ≥ 阈值 → 进入生成
└──────────┬───────────┘
           ↓
┌──────────────────────┐
│ 阶段六：报告生成       │  Writer Agent → 渐进式构建
│  (渐进式)             │  Verifier → 最终事实核查
└──────────┬───────────┘
           ↓
     最终研究报告 (含引用图 + 质量评分)
```

## 核心创新点

1. **质量驱动的自适应迭代** — 四维质量评分（覆盖度/可靠性/新鲜度/深度）驱动迭代终止
2. **三层记忆架构** — 工作记忆 / 情节记忆 (RocksDB) / 语义记忆 (RocksDB + 向量)
3. **引用图与交叉验证** — petgraph 二部图 + 多源验证
4. **渐进式报告生成** — 每轮迭代更新报告草稿
5. **成本感知路由** — 任务复杂度 → 自动选择模型层级

## Crate 设计

| Crate | 职责 | 关键依赖 |
|-------|------|---------|
| `zhiyuan-core` | 核心类型、trait、错误 | serde, thiserror |
| `zhiyuan-search` | 搜索引擎抽象 (Bing/DDG/Google) | reqwest, scraper |
| `zhiyuan-extract` | 网页内容提取 | scraper, reqwest |
| `zhiyuan-memory` | 三层记忆 (RocksDB) | rocksdb, serde |
| `zhiyuan-orchestrator` | 编排层、质量评估 | petgraph, rig |
| `zhiyuan-agents` | 六类智能体 | rig, zhiyuan-* |
| `zhiyuan-robust` | 重试、规范化、故障隔离 | tokio, backoff |
