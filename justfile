# 致远 — 快捷命令
# 运行：just <task>

default: run

# 运行主程序
run *query:
    cargo run -- "{{query}}"

# 编译（不含测试）
build:
    cargo build

# 快速检查编译
check:
    cargo check

# 检查代码格式
fmt:
    cargo fmt --check

# 自动格式化
fmt-fix:
    cargo fmt

# 单元测试（搜索 + 提取 crate）
unit:
    cargo test -p zhiyuan-search -p zhiyuan-extract --lib

# 全量单元测试（含远端编译慢的 crate）
unit-all:
    cargo test --workspace --lib

# 流水线集成测试（需要网络）
it:
    cargo test -p zhiyuan-tests --test pipeline -- --ignored --nocapture

# 单个引擎集成测试，例如 just it-engine bing
it-engine engine:
    cargo test -p zhiyuan-tests --test pipeline test_{{engine}}_pipeline -- --ignored --nocapture

# 运行全部测试（单元 + 集成）
test: unit
    cargo test -p zhiyuan-tests --test pipeline test_bing_pipeline -- --ignored --nocapture

# 发布构建
release:
    cargo build --release

# ────────────── SearXNG ──────────────

# 启动 SearXNG（容器已存在则跳过）
searxng-up:
    docker compose -f searxng/docker-compose.yml up -d

# 停止 SearXNG
searxng-down:
    docker compose -f searxng/docker-compose.yml down

# 重启 SearXNG
searxng-restart:
    docker compose -f searxng/docker-compose.yml restart

# 查看 SearXNG 日志
searxng-logs:
    docker compose -f searxng/docker-compose.yml logs -f

# SearXNG 运行状态
searxng-status:
    docker compose -f searxng/docker-compose.yml ps
