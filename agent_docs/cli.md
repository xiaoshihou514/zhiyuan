# 致远 CLI 接口文档

## 基本用法

```bash
cargo run -- --query "<研究问题>" [选项]
```

## 命令行参数

| 参数                  | 类型   | 默认值           | 描述                                        |
| --------------------- | ------ | ---------------- | ------------------------------------------- |
| `-q`, `--query`       | String | **必填**         | 研究问题                                    |
| `--interactive`       | bool   | `true`           | 研究前 LLM 生成澄清问题并等待用户回答       |
| `--long-report`       | bool   | `false`          | 启用长报告模式（多章节结构报告）            |
| `--max-chapters`      | usize  | `6`              | 长报告最大章节数 (3–10)                     |
| `--quality-threshold` | f64    | `0.7`            | 质量阈值 (0.0–1.0)，达到后提前停止迭代      |
| `--max-iterations`    | usize  | `10`             | 最大质量迭代轮数                            |
| `--breadth`           | usize  | `4`              | 搜索广度（每轮并行查询数）                  |
| `--depth`             | usize  | `3`              | 搜索深度（递归层数）                        |
| `--concurrency`       | usize  | `4`              | 任务并发数                                  |
| `-c`, `--config`      | String | —                | 配置文件路径（默认: `config/default.toml`） |
| `-d`, `--data-dir`    | String | `./zhiyuan_data` | RocksDB 数据目录（记忆存储）                |
| `-o`, `--output`      | String | —                | 输出 JSON 文件路径（不指定则打印到 stdout） |

### 示例

```bash
# 基本研究（交互模式默认开启）
cargo run -- --query "Rust 2026 异步生态发展"

# 跳过交互澄清
cargo run -- --query "量子计算最新突破" --interactive false

# 长报告模式
cargo run -- --query "光伏电池技术路线对比" --long-report --max-chapters 5

# 高质量深度研究
cargo run -- --query "RISC-V AI 加速器架构" \
  --quality-threshold 0.9 --max-iterations 20 --breadth 8 --concurrency 6

# 输出到文件
cargo run -- --query "WebAssembly 在边缘计算中的应用" -o report.json
```

## 环境变量

| 变量              | 必填   | 默认值           | 描述                                             |
| ----------------- | ------ | ---------------- | ------------------------------------------------ |
| `OPENAI_API_KEY`  | **是** | —                | OpenAI API 密钥                                  |
| `MAIN_MODEL`      | 否     | `gpt-4o`         | 主模型（智能体使用）                             |
| `REASONING_MODEL` | 否     | `gpt-4o`         | 推理模型                                         |
| `FAST_MODEL`      | 否     | `gpt-4o-mini`    | 快速模型                                         |
| `BING_API_KEY`    | 否*    | —                | Bing Search API 密钥                             |
| `BING_ENDPOINT`   | 否     | Bing v7 默认端点 | Bing 搜索端点                                    |
| `GOOGLE_API_KEY`  | 否*    | —                | Google Custom Search API 密钥                    |
| `GOOGLE_CSE_ID`   | 否*    | —                | Google CSE 搜索引擎 ID                           |
| `RUST_LOG`        | 否     | `info`           | 日志级别 (`error`/`warn`/`info`/`debug`/`trace`) |

> *至少配置一个搜索引擎（Bing / Google / DuckDuckGo），DuckDuckGo 无需 API 密钥。

`.env` 文件会被自动加载（如果存在）。

## 交互模式

`--interactive true`（默认）时流程：

```
1. CLI 读取 --query
2. PlannerAgent 调用 LLM 生成 2–4 个澄清问题
3. 终端逐题展示，等待用户输入回答
4. 回答合并为 clarification 上下文 → ResearchQuery
5. ResearcherOrchestrator 开始研究
```

用户可直接回车跳过某个问题，所有问题跳过则 `clarification` 为 `None`。

## 长报告模式

`--long-report` 启用时：

1. `PlannerAgent::create_long_plan` 生成多章节大纲 + 逐章子任务
2. 迭代收集研究发现
3. 研究结束后 `assign_findings_to_chapters` 按章节分配发现
4. `cross_check_chapters` 检测章节间重复/矛盾
5. `WriterAgent::write_long_report` 编译完整长报告

## 输出格式

### stdout（默认）

短报告模式打印章节标题 + Markdown 内容 + 质量评分。
长报告模式打印完整报告标题 + 各章节内容 + 质量评分。

### JSON 文件（`-o` 指定）

```json
{
  "query_id": "uuid",
  "title": "研究问题 - 研究报告",
  "sections": [
    {
      "heading": "章节标题",
      "content": "Markdown 内容",
      "citations": ["https://..."]
    }
  ],
  "citation_graph": { "claims": [], "sources": [], "edges": [] },
  "quality_score": {
    "coverage": 0.85,
    "reliability": 0.72,
    "freshness": 0.68,
    "depth": 0.79,
    "overall": 0.77
  },
  "generated_at": "2026-07-15T12:00:00Z"
}
```

## 语义记忆

数据存储在 `--data-dir` 指定的 RocksDB 数据库中，包含三层：

- **working**: 当前研究会话的工作数据
- **episodic**: 按研究 ID + 迭代编号存储的原始发现
- **semantic**: 按主题存储的实体关系 + 研究发现

后续研究启动时会自动查询 semantic memory 中与 `--query` 关键词匹配的历史发现，作为初始上下文注入。

## 退出码

| 退出码 | 含义          |
| ------ | ------------- |
| 0      | 成功          |
| 1      | 配置/参数错误 |
| 2      | 研究执行失败  |
