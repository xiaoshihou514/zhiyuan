# 致远 CLI 接口文档

## 基本用法

```bash
cargo run -- "<研究问题>" [选项]
```

## 命令行参数

| 参数            | 类型   | 默认值 | 描述                                             |
| --------------- | ------ | ------ | ------------------------------------------------ |
| `query`         | String | **必填** | 研究问题（positional argument）                 |
| `--clarify`     | bool   | `true` | 研究前 LLM 生成澄清问题并等待用户回答            |
| `--long`        | bool   | `false` | 启用长报告模式（多章节结构报告，自动开启交叉验证） |
| `--concurrency` | usize  | `4`    | 任务并发数                                       |

### 示例

```bash
# 基本研究（交互澄清默认开启）
cargo run -- "Rust 2026 异步生态发展"

# 跳过交互澄清
cargo run -- "量子计算最新突破" --clarify false

# 长报告模式（自动开启交叉验证）
cargo run -- "光伏电池技术路线对比" --long

# 高并发深度研究
cargo run -- "RISC-V AI 加速器架构" --concurrency 8
```

## 环境变量

| 变量              | 必填 | 默认值 | 描述                                                |
| ----------------- | ---- | ------ | --------------------------------------------------- |
| `OPENAI_API_KEY`  | 否   | —      | 覆盖配置文件中 [llm] 的 api_key                     |
| `OPENAI_BASE_URL` | 否   | —      | 覆盖配置文件中 [llm] 的 base_url                    |
| `RUST_LOG`        | 否   | `info` | 日志级别 (`error`/`warn`/`info`/`debug`/`trace`)    |

搜索引擎无需 API 密钥——Bing、Google、DuckDuckGo 均通过 HTML 解析获取结果。

`.env` 文件会被自动加载（如果存在）。

## 交互模式

`--clarify true`（默认）时流程：

```
1. CLI 读取 positional query
2. PlannerAgent 调用 LLM 生成 2–4 个澄清问题
3. 终端逐题展示，等待用户输入回答
4. 回答合并为 clarification 上下文 → ResearchQuery
5. ResearcherOrchestrator 开始研究
```

用户可直接回车跳过某个问题，所有问题跳过则 `clarification` 为 `None`。

## 长报告模式

`--long` 启用时，自动开启交叉验证：

1. `PlannerAgent::create_long_plan` 生成多章节大纲（LLM 自行决定章节数量）+ 逐章子任务
2. 迭代收集研究发现，每轮经 LLM 事实核查
3. 研究结束后按章节分配发现，检测章节间重复/矛盾
4. `WriterAgent::write_long_report` 编译完整长报告

## 输出格式

短报告模式打印章节标题 + Markdown 内容 + 质量评分。
长报告模式打印完整报告标题 + 各章节内容 + 质量评分。

报告标题由 LLM 根据研究内容自动生成。

## 交叉验证 (`--cross-validate`)

`--long` 模式下自动开启。独立使用时需在配置文件的 `[research]` 中设置 `cross_validate = true`。

启用后：

1. 所有搜索引擎**并行**执行查询，记录每个结果被多少个独立引擎发现
2. 结果按跨引擎覆盖数排序（多引擎共同发现的排前面）
3. 使用 LLM 对每条研究发现进行事实核查，过滤掉不可靠或仅有单一来源的内容

```
[info] cross-search contributed engine=bing count=10
[info] cross-search contributed engine=google count=8
[info] cross-search contributed engine=duckduckgo count=7
[info] cross-search completed engine_count=3 total_results=18
[info] 交叉验证: 2/12 个发现被过滤
```

适用场景：对信息准确性要求高的研究，需要多重验证保障事实可靠性。

## 自适应搜索查询规划

查询生成由 `QueryPlannerAgent` 独立完成，它会根据子任务内容自适应决定查询语言策略：

- 识别子任务中的技术术语、英文专有名词、框架/库名称
- 自动判断是否需要混合使用中英文查询
- 对含大量英文术语的任务自动生成英文查询

例如，对于"Rust 异步运行时对比"，可能会自动生成：
- "Rust tokio vs async-std vs smol 性能对比"
- "Rust async runtime benchmark comparison"
- "tokio async-std smol feature comparison"

纯中文主题则全部使用中文查询，无需手动指定 `--search-in-english`。

## 语义记忆

数据存储在 `~/.cache/zhiyuan/<query_hash>` 的 RocksDB 数据库中，包含三层：

- **working**: 当前研究会话的工作数据
- **episodic**: 按研究 ID + 迭代编号存储的原始发现
- **semantic**: 按主题存储的实体关系 + 研究发现

后续研究启动时会自动查询 semantic memory 中与查询匹配的历史发现，作为初始上下文注入。

## 配置

配置查找顺序：`~/.config/zhiyuan.toml` → `./zhiyuan.toml`。

参考项目中的 `zhiyuan.toml.example` 创建配置文件。

## 退出码

| 退出码 | 含义          |
| ------ | ------------- |
| 0      | 成功          |
| 1      | 配置/参数错误 |
| 2      | 研究执行失败  |
