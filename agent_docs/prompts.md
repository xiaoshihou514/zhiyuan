# LLM 提示词模板概况

所有 prompt 均为**硬编码中文**，嵌入在 Rust 源码中。temperature 统一为 **0.3**（`src/llm.rs:114`）。

## 提示词分类

| # | 位置（文件:行） | 角色 | system prompt 要点 | 输出要求 |
|---|---------------|------|-------------------|---------|
| 1 | `planner.rs:16-18` | 生成澄清问题 | "研究助理…请生成 2-4 个澄清性问题" | 每行一个问题 |
| 2 | `planner.rs:51-53` | 短报告规划 | "研究规划专家…将复杂问题分解为子任务" | 纯 JSON |
| 3 | `planner.rs:86-88` | 长报告规划 | "研究规划和报告结构专家…生成多章节大纲" | 纯 JSON |
| 4 | `query_planner.rs:18-27` | 搜索查询规划 | "搜索查询规划专家…每个查询 2-3 个高热度词…多语言…SearXNG 类别" | 纯 JSON |
| 5 | `synthesizer.rs:68-70` | 信息综合 | "信息综合专家…与已有发现比对，只关注新信息/矛盾点/补充细节" | 纯 JSON |
| 6 | `synthesizer.rs:136-138` | 摘要生成 | "研究摘要专家…每条摘要 100-200 字" | 纯 JSON |
| 7 | `synthesizer.rs:208-210` | 研究方向识别 | "研究方向识别专家…优先关注未覆盖的子任务" | 纯 JSON |
| 8 | `verifier.rs:38-40` | 事实核查 | "事实核查专家…交叉验证声明…支持或矛盾" | 纯 JSON |
| 9 | `writer.rs:128-133` | 报告更新 | "根据已有报告草稿和新的研究发现，更新和优化…Typst 格式…@key 引用" | 纯 Typst |
| 10 | `writer.rs:206-210` | 长报告写作 | "根据多章节大纲和发现…Typst 格式…@key 引用" | 纯 Typst |
| 11 | `writer.rs:328-332` | 报告内容构建 | "根据研究发现和引用信息…Typst 格式…@key 引用" | 纯 Typst |
| 12 | `orchestrator.rs:535-543` | 交叉验证 | "事实核查专家…TRUE 或 FALSE" | `TRUE` / `FALSE` |
| 13 | `orchestrator.rs:766` | 章节分配 | "研究分析专家…将每个发现分配到最合适的章节" | 纯 JSON |
| 14 | `orchestrator.rs:850` | 章节交叉校对 | "研究报告校对专家…重复/矛盾/遗漏" | 自由文本 |
| 15 | `main.rs:479-485` | Typst 错误修复 | Typst 编译错误→LLM 修复对应段落（含 Levenshtein bib key 建议） | 纯 Typst |

## 设计模式

### 1. JSON 输出约定

除 writer 和交叉验证外，所有 prompt 要求**纯 JSON**，明确指定格式：

```
只输出纯 JSON，不要 markdown 格式、不要代码块、不要其他文字。
```

输出示例嵌入在 user prompt 中：

```
输出 JSON 格式：{"sub_tasks": [{"description": "...", "dependencies": []}]}
```

LLM 响应通过 `extract_json()`（`zhiyuan-agents/src/util.rs`）后处理：
- 去除 ` ```json ` / ` ``` ` 围栏
- 找到第一个 `{` 或 `[` 及匹配的闭合括号

### 2. Typst 输出约定

WriterAgent 和错误修复使用纯 Typst 输出：

```
只输出纯 Typst 正文，不要 ```typst 围栏。
第一行必须是 = 开头的一级标题（报告标题）。
不要生成参考文献/参考资料章节，系统会自动添加。
```

引用使用 `@key` 格式，由 `bib_key()` 函数（`writer.rs:8-40`）生成：
- 格式：`{domain_prefix}_{path_slug_first_12_chars}`
- 如 `kpmg_report23`, `arxiv_2103.12345`

### 3. 上下文窗口策略

| Agent | 输入截断 |
|-------|---------|
| Synthesizer.synthesize | 每个内容截取前 1500 字符；总长>5000 时先 summarize |
| Synthesizer.summarize | 每个摘要 100-200 字 |
| ExtractorAgent | 片段匹配（CJK bigram/trigram + ASCII 分词） |
| SearcherAgent | 已有发现拼接为 context，截断到前 1000 字符 |
| WriterAgent | 通过 key_map_table 传递 bib key 对照表 |

### 4. 错误降级

每个 Agent 都有 JSON 解析失败的回退逻辑：

| Agent | 降级策略 |
|-------|---------|
| PlannerAgent | 单任务计划（直接使用用户查询） |
| QueryPlannerAgent | 任务描述本身作为搜索词，类别="general" |
| SynthesizerAgent | 前 3 个来源各 500 字作为 finding |
| VerifierAgent | 空边表 `{"edges": []}` |
| WriterAgent | 保留已有报告（update_report），或回退到简单标题 |

### 5. 语言策略

- 所有 system prompt 为中文
- User prompt 中嵌入的中英文由上游数据决定
- QueryPlanner 被告知可自动选择查询语言（包括法日德等）
