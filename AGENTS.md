# 致远 (Zhiyuan) — 深度研究框架

## Build & Run

```bash
cargo build
cargo run -- "你的研究问题"
cargo run -- "问题" --long                         # 长报告模式
cargo run -- "问题" --no-clarify                    # 跳过澄清
cargo run -- "问题" --concurrency 8                 # 调并发数
```

## Key Architecture

- **Binary entry**: `src/main.rs` → `zhiyuan` binary
- **7 workspace crates** under `crates/`, resolver = "2"
- **DAG**: `core` ← `search`, `extract`, `memory`, `robust` ← `agents` ← `orchestrator`
- **In-process Typst PDF**: uses `typst` + `typst-pdf` crates, needs CJK fonts (`Noto Sans CJK SC` recommended)
- **Search engines**: Bing/Google/DDG via HTML scraping (no API keys needed), `EnginePool` with fallback
- **LLM client**: OpenAI-compatible, `base_url` + `main_model` configurable, default `gpt-4o`
- **Memory**: RocksDB with 3 column families (`working`, `episodic`, `semantic`) at `~/.cache/zhiyuan/<query_hash>/`

## Config & Data

| Path | Purpose |
|------|---------|
| `~/.config/zhiyuan.toml` or `./zhiyuan.toml` | App config (copy from `zhiyuan.toml.example`) |
| `.env` | dotenv (optional) |
| `~/.cache/zhiyuan/<hash>/` | RocksDB memory store |
| `~/.local/share/zhiyuan/<hash>.log` | Tracing output |

## Commits

```
git commit -m "feat: 消息" -m "Co-authored-by: opencode <deepseek@opencode.com>"
```

All commits as `xiaoshihou <xiaoshihou@tutamail.com>`.
