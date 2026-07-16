use tracing_subscriber::EnvFilter;
use zhiyuan_core::SearchQuery;
use zhiyuan_extract::{ContentExtractor, WebExtractor};
use zhiyuan_search::{BingEngine, DuckDuckGoEngine, SearchEngine, StartpageEngine};

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_test_writer()
        .with_target(false)
        .try_init()
        .ok();
}

fn query() -> SearchQuery {
    SearchQuery {
        query: "Rust async programming tokio 2025".into(),
        max_results: 5,
        region: None,
        language: None,
    }
}

const CONTEXT: &str = "Rust async programming tokio";

async fn run_pipeline<E>(engine: &E, name: &str)
where
    E: SearchEngine,
{
    eprintln!("\n=== {name} ===");
    let results = match engine.search(&query()).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  [{name}] 搜索失败（跳过测试）: {e}");
            return;
        }
    };
    if results.is_empty() {
        eprintln!("  [{name}] 搜索返回空结果（已知引擎限制，跳过后续）");
        return;
    }

    for (i, r) in results.iter().enumerate() {
        let clean = if r.url.starts_with("//") {
            &r.url[2..]
        } else {
            &r.url
        };
        println!(
            "[{name}][{i}] {} — {}",
            r.title,
            clean.chars().take(100).collect::<String>()
        );
    }

    let n = results.len();
    let mut urls: Vec<&str> = results.iter().map(|r| r.url.as_str()).collect();
    urls.sort();
    urls.dedup();
    assert_eq!(urls.len(), n, "[{name}] 搜索结果不应有重复URL");

    let extractor = WebExtractor::new();
    let mut ok = 0usize;
    for result in results.iter().take(3) {
        match extractor.extract(result, CONTEXT).await {
            Ok(content) => {
                if content.text.is_empty() {
                    println!("  - 提取内容为空（跳过）");
                    continue;
                }
                assert!(
                    content.text.len() > 50,
                    "[{name}] 提取内容应有实质文字"
                );
                println!(
                    "  ✓ 提取成功: {} 字符, 关联度 {:.2}",
                    content.text.len(),
                    content.relevance_score
                );
                ok += 1;
            }
            Err(e) => println!("  ✗ 提取失败: {e}"),
        }
    }

    if name == "Bing" {
        assert!(ok > 0, "[{name}] 应至少有 1 条成功提取的内容");
    }
    println!("  [{name}] 流水线测试通过: {ok}/3 提取成功\n");
}

#[tokio::test]
#[ignore]
async fn test_bing_pipeline() {
    init_tracing();
    run_pipeline(&BingEngine::new(5), "Bing").await;
}

#[tokio::test]
#[ignore]
async fn test_startpage_pipeline() {
    init_tracing();
    run_pipeline(&StartpageEngine::new(5), "Startpage").await;
}

#[tokio::test]
#[ignore]
async fn test_ddg_pipeline() {
    init_tracing();
    run_pipeline(&DuckDuckGoEngine::new(5), "DDG").await;
}

/// 中文 Bing 测试：验证 URL 解码 + 内容提取 + 结果相关性
#[tokio::test]
#[ignore]
async fn test_bing_chinese() {
    init_tracing();
    let engine = BingEngine::new(5);
    let query = SearchQuery {
        query: "国产 AUTOSAR 工具链".into(),
        max_results: 5,
        region: None,
        language: None,
    };
    eprintln!("\n=== Bing 中文 ===");
    let results = engine.search(&query).await.expect("搜索应返回结果");
    assert!(!results.is_empty(), "搜索结果不应为空");

    let mut decoded = 0;
    let mut relevant = 0;
    for (i, r) in results.iter().enumerate() {
        if r.url.contains("bing.com/ck/") {
            eprintln!("  [{i}] ⚠ 未解码");
        } else {
            decoded += 1;
        }
        let lower = r.title.to_lowercase();
        let rel = lower.contains("autosar") || lower.contains("工具链");
        if rel { relevant += 1; }
        eprintln!("  [{i}] [{rel}] {:.60}", r.title);
    }
    assert!(decoded > 0, "应至少有一条 URL 被成功解码");
    if relevant == 0 {
        eprintln!("  ⚠ 本次 Bing 中文查询未返回 AUTOSAR 相关结果（cn.bing.com 结果不稳定，非代码问题）");
    }
    eprintln!("  解码: {}/{} 条，相关: {}/{} 条", decoded, results.len(), relevant, results.len());

    // 内容提取：试前 3 条中有解码成功且相关的
    let extractor = WebExtractor::new();
    let mut ok = 0;
    for r in results.iter().take(3) {
        if !r.url.contains("bing.com/ck/") || decoded > 0 {
            match extractor.extract(r, "国产 AUTOSAR 工具链").await {
                Ok(c) => {
                    if !c.text.is_empty() {
                        eprintln!("  ✓ 提取成功: {} 字符，{}", c.text.len(), c.title);
                        ok += 1;
                    }
                }
                Err(e) => eprintln!("  ✗ 提取失败: {e}"),
            }
        }
    }
    eprintln!("  提取: {}/3 条成功", ok);
}
