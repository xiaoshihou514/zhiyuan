# 智能体设计（7 个 LLM 驱动 Agent）

所有智能体均通过 `LlmClient::prompt(system, user)` 直接调用 LLM，不依赖任何 Agent 框架。输出统一用 `extract_json()` 从 LLM 响应中提取纯 JSON（去除 markdown 围栏）。

## 一览表

| 智能体 | 文件 | 职责 | 输出格式 |
|--------|------|------|---------|
| PlannerAgent | `planner.rs` | 生成澄清问题、研究计划（短/长） | JSON: `{sub_tasks, outline?}` |
| QueryPlannerAgent | `query_planner.rs` | 为子任务生成 2-4 个搜索查询 + SearXNG 分类 | JSON: `{categories, queries}` |
| SearcherAgent | `searcher.rs` | 并发执行搜索（fallback / cross-validate 模式） | `Vec<SearchResult>` |
| ExtractorAgent | `extractor.rs` | 域名过滤 + 片段匹配评分 + 选择性提取 top-10 URL | `Vec<ExtractedContent>` |
| SynthesizerAgent | `synthesizer.rs` | 新内容 vs 已有发现比对、摘要生成、方向识别 | `Vec<Finding>`, `Vec<ResearchDirection>` |
| VerifierAgent | `verifier.rs` | 声明-来源 支持/矛盾关系构建 | `CitationGraph` |
| WriterAgent | `writer.rs` | Typst 报告生成/更新/长报告编译 | `ResearchReport` |

## PlannerAgent

- **`generate_clarifying_questions(query)`** — 生成 2-4 个问题，覆盖时间范围、地域、具体领域、关注重点
- **`create_plan(query, settings)`** — 根据 `settings.long_report` 分流：
  - **短报告**：分解为 3-6 个子任务，纯 JSON 输出
  - **长报告**：生成 3-8 章大纲 + 每章 2-3 个子任务（含 `chapter_index`）

## QueryPlannerAgent

- **`plan_queries(task_description, context)`** — 核心搜索查询规划
  - 生成 2-4 个查询，每个 2-3 个高热度词
  - 自动选择查询语言（中英以外可覆盖法日德等）
  - 选择 SearXNG 类别：`science` / `general` / `news` / `it`（可组合，逗号分隔）
  - 后处理：去重、截断到 5 词

## SearcherAgent

- **`generate_queries(task_desc, context)`** → 调用 QueryPlannerAgent
- **`execute_search(queries, max_results, concurrency, cross_validate, categories)`**：
  - 每查询并发执行，通过 Semaphore 控制并发
  - `cross_validate=true` → 调用 `engine.search_all()`（并发所有引擎 + 去重排序）
  - `cross_validate=false` → 调用 `engine.search()`（fallback 顺序）

## ExtractorAgent

- **域名过滤**：`blocked_domains` 列表匹配
- **片段提取**：`extract_fragments(context)` — CJK/ASCII 边界切分，对 CJK 多字词生成重叠 bigram/trigram
- **优先级评分**：`result_priority_score()` — 标题+摘要中片段匹配比例
- **提取**：取 top-10 最高分 URL，每 32 个一批并发调用 `ContentExtractor.extract()`

## SynthesizerAgent

- **`synthesize(contents, sub_task_id, iteration, existing_findings)`**：
  - 长内容先调用 `summarize()` 压缩
  - 与已有发现比对 → 输出"新信息、矛盾点、补充细节"
  - JSON 解析失败时降级：取前 3 个来源各 500 字符作为 finding
- **`summarize(content)`** — 每条信息生成 100-200 字摘要
- **`extract_directions(question, findings, sub_tasks)`** — 识别 1-3 个知识盲区/待探索方向（含优先级 0-1）

## VerifierAgent

- **`verify_claims(claims, sources)`** — 对每个声明判断各来源是"支持"还是"矛盾"
  - 输出 `CitationGraph`（`claims: Vec<Claim>` + `sources: Vec<SourceNode>` + `edges: Vec<CitationEdge>`）
  - 每个 `CitationEdge` 包含 `claim_id`, `source_id`, `relation`（supports / contradicts）

## WriterAgent

- **`write_report()`** — 从零生成短报告（Typst 格式）
- **`update_report()`** — 渐进式更新：已有报告 + 新发现 → 输出更新版 Typst
- **`write_long_report()`** — 按大纲 + 各章节发现 + 交叉校对意见 → 输出完整 Typst 长报告
- **`build_report_content()`** — 通用报告内容构建（4 节：摘要、背景、主要发现、结论与展望）
- 引用格式：`@key` 格式（如 `@kpmg_report23`），系统自动添加参考文献章节
- 第一行必须是 `= ` 开头的一级标题

## 公共工具 (`util.rs`)

- **`extract_json()`** — 从 LLM 响应中提取纯 JSON（去除 ```json 围栏，找到第一个 `{`/`[`）
