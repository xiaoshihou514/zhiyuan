# 致远 CLI 接口文档

## 基本用法

```bash
cargo run -- "<研究问题>" [选项]
cargo build --release
./target/release/zhiyuan "<研究问题>"
```

## 命令行参数

| 参数 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `query` | String | **必填** | 研究问题（positional argument） |
| `--clarify` | bool | `true` | 研究前 LLM 生成澄清问题并等待用户回答 |
| `--long` | bool | `false` | 启用长报告模式（多章节结构报告，自动开启交叉验证） |
| `--concurrency` | usize | `4` | 任务并发数 |

### 示例

```bash
# 基本研究（交互澄清默认开启）
cargo run -- "Rust 2026 异步生态发展"

# 跳过交互澄清
cargo run -- "量子计算最新突破" --no-clarify

# 长报告模式（自动开启交叉验证）
cargo run -- "光伏电池技术路线对比" --long

# 高并发深度研究
cargo run -- "RISC-V AI 加速器架构" --concurrency 8
```

*注意：`--clarify` 实际通过 clap 的 `long` 标志控制，默认 true，使用 `--no-clarify` 关闭。*

## 环境变量

| 变量 | 必填 | 默认值 | 描述 |
|------|------|--------|------|
| `RUST_LOG` | 否 | `info,html5ever=off,pdf_oxide=off,...` | 日志级别 |
| `HOME` | 否 | 系统 HOME | 决定 session/log 目录基路径 |

- 搜索引擎使用 **SearXNG**（需本地运行），无需外部 API 密钥
- LLM API 密钥通过配置文件设置，不支持环境变量覆盖
- 日志文件写入 `~/.local/share/zhiyuan/<query_hash>.log`
- `.env` 文件会被自动加载（如果存在）

## 交互模式

`--clarify`（默认开启）时流程：

```
1. CLI 读取 positional query
2. PlannerAgent 调用 LLM 生成 2-4 个澄清问题
3. TUI 逐题展示，等待用户输入回答
4. 回答合并为 clarification 上下文 → ResearchQuery
5. ResearchOrchestrator 开始研究
```

用户可直接回车跳过某个问题，所有问题跳过则 clarification 为 None。

## 长报告模式

`--long` 启用时，自动开启交叉验证：

1. `PlannerAgent::create_long_plan` 生成多章节大纲（LLM 决定章节数量 3-8 个）+ 逐章子任务
2. 迭代收集研究发现，每轮经 VerifierAgent 事实核查
3. 研究结束后按章节分配发现，LLM 检测章节间重复/矛盾
4. `WriterAgent::write_long_report` 编译完整 Typst 长报告
5. 短报告上限 3 迭代轮次，长报告上限 3 轮

## 交叉验证模式

`--long` 模式下自动开启。也可在配置 `[research]` 中设置 `cross_validate = true`。

启用后：
1. 所有搜索引擎**并行**执行查询（`search_all()`），结果按跨引擎覆盖数排序
2. 每发现用 LLM 逐条判断 TRUE/FALSE（相似度 ≥0.3 跳过 LLM 节省 token）
3. 被过滤的发现记录日志

## 输出格式

报告在 TUI 中展示，包含：
- 报告标题（LLM 根据内容自动生成）
- 各章节正文（Typst 格式源码预览）
- 质量评分（覆盖度/可信度/信息量/新颖度）
- 可选 PDF 导出

PDF 生成流程：
```
Typst 源码 → typst::compile → PagedDocument → typst-pdf
出错 → LLM 修复（最多 5 轮）
```

## 配置

配置查找顺序：`~/.config/zhiyuan.toml` → `./zhiyuan.toml`。

参考项目中的 `zhiyuan.toml.example` 创建配置文件。关键配置：

| 节 | 键 | 默认值 | 说明 |
|----|----|--------|------|
| `[search]` | `searxng_url` | `http://localhost:8888` | SearXNG 实例 |
| `[llm]` | `base_url` | `https://api.deepseek.com/v1` | OpenAI 兼容端点 |
| `[llm]` | `main_model` | `deepseek-v4-flash` | 模型名 |
| `[research]` | `max_iterations` | `4` | 最大迭代轮次 |

## 退出码

| 退出码 | 含义 |
|--------|------|
| 0 | 成功 |
| 1 | 配置/参数错误 |
| 2 | 研究执行失败 |
