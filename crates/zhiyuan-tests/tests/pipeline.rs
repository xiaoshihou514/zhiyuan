use tracing_subscriber::EnvFilter;
use zhiyuan_core::SearchQuery;
use zhiyuan_extract::{ContentExtractor, WebExtractor};
use zhiyuan_search::{SearXngEngine, SearchEngine};

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_test_writer()
        .with_target(false)
        .try_init()
        .ok();
}

const SEARXNG_URL: &str = "http://localhost:8888";

async fn run_pipeline<E>(engine: &E, name: &str)
where
    E: SearchEngine,
{
    eprintln!("\n=== {name} ===");
    let query = SearchQuery {
        query: "Rust async programming tokio".into(),
        max_results: 5,
        region: None,
        language: None,
    };
    let results = match engine.search(&query).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  [{name}] 搜索失败（跳过测试）: {e}");
            return;
        }
    };
    if results.is_empty() {
        eprintln!("  [{name}] 搜索返回空结果（跳过）");
        return;
    }

    for (i, r) in results.iter().enumerate() {
        println!("[{name}][{i}] {} — {}", r.title, r.url.chars().take(100).collect::<String>());
    }

    let n = results.len();
    let mut urls: Vec<&str> = results.iter().map(|r| r.url.as_str()).collect();
    urls.sort();
    urls.dedup();
    assert_eq!(urls.len(), n, "[{name}] 搜索结果不应有重复URL");

    let extractor = WebExtractor::new();
    let context = "Rust async programming tokio";
    let mut ok = 0usize;
    for result in results.iter().take(3) {
        match extractor.extract(result, context).await {
            Ok(content) => {
                if content.text.is_empty() { continue; }
                if content.text.len() <= 50 { continue; }
                println!("  ✓ 提取成功: {} 字符, 关联度 {:.2}", content.text.len(), content.relevance_score);
                ok += 1;
            }
            Err(e) => println!("  ✗ 提取失败: {e}"),
        }
    }
    println!("  [{name}] 流水线测试通过: {ok}/3 提取成功\n");
}

#[tokio::test]
#[ignore]
async fn test_searxng_pipeline() {
    init_tracing();
    let engine = SearXngEngine::new(SEARXNG_URL, 5);
    run_pipeline(&engine, "SearXNG").await;
}
