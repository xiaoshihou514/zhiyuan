use std::collections::{HashMap, HashSet};
use zhiyuan_core::{KnowledgeBase, LlmClient, QualityScore, ResearchPlan};

pub struct QualityEvaluator {
    llm: Option<Box<dyn LlmClient>>,
}

impl QualityEvaluator {
    pub fn new(llm: Option<Box<dyn LlmClient>>) -> Self {
        Self { llm }
    }

    pub fn evaluate(
        &self,
        knowledge: &KnowledgeBase,
        query: &str,
        plan: &ResearchPlan,
        _citation_graph: &zhiyuan_core::CitationGraph,
    ) -> QualityScore {
        let coverage = self.calc_coverage(knowledge, plan, query);
        let reliability = self.calc_reliability(knowledge);
        let freshness = self.calc_freshness(knowledge);
        let depth = self.calc_depth(knowledge);

        QualityScore::new(coverage, reliability, freshness, depth)
    }

    /// 三维可靠性
    ///
    /// 1. 来源权威性：LLM 评估 URL 域名权威性（回退关键词规则）
    /// 2. 内部一致性：同源声明是否自洽（Jaccard 相似度检测矛盾）
    /// 3. 语义一致性：同子任务的发现是否指向相同结论
    fn calc_reliability(&self, knowledge: &KnowledgeBase) -> f64 {
        if knowledge.findings.is_empty() {
            return 0.0;
        }

        let authority = self.dim_authority(knowledge);
        let consistency = self.dim_consistency(knowledge);
        let semantic = self.dim_semantic(knowledge);

        // 等权平均
        (authority + consistency + semantic) / 3.0
    }

    /// 1. 来源权威性：LLM 评估 URL 域名权威性（回退关键词规则）
    fn dim_authority(&self, knowledge: &KnowledgeBase) -> f64 {
        // 收集所有唯一 URL
        let mut unique_urls: Vec<&str> = Vec::new();
        let mut seen = HashSet::new();
        for f in &knowledge.findings {
            for url in &f.sources {
                if seen.insert(url.as_str()) {
                    unique_urls.push(url);
                }
            }
        }

        if unique_urls.is_empty() {
            return 0.3;
        }

        // 优先用 LLM 评估
        if let Some(ref llm) = self.llm {
            return self.llm_authority(&**llm, &unique_urls);
        }

        // 回退：关键词规则
        self.keyword_authority(&unique_urls)
    }

    /// LLM 评估 URL 权威性（system prompt 固定，优化 KV 缓存）
    fn llm_authority(&self, llm: &dyn LlmClient, urls: &[&str]) -> f64 {
        let system = "\
你是一个来源权威性评估专家。对每个 URL，根据其域名判断信息权威性，返回 0-1 的分数。

评分标准：
1.0  顶级学术机构（.edu/.gov 域名）、学术出版商（IEEE/ACM/Springer/Elsevier/arXiv）
0.8  技术文档/标准组织（W3C/MDN/Rust-lang/Kernel.org）、代码托管（GitHub/GitLab）
0.6  知名技术媒体/社区（Medium/Stack Overflow/InfoQ/DZone/O'Reilly）
0.4  普通商业网站、个人博客、新闻媒体
0.2  低质量内容农场、论坛、自媒体、SEO 站点

只输出纯 JSON，不要 markdown 围栏，不要其他文字。";

        let url_list: String = urls
            .iter()
            .enumerate()
            .map(|(i, u)| format!("{}. {}", i + 1, u))
            .collect::<Vec<_>>()
            .join("\n");

        let user = format!("请评估以下 URL 的权威性分数：\n{url_list}\n\n输出 JSON 格式：{{\"scores\": [{{\"url\": \"...\", \"score\": 0.0}}]}}");

        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let response = handle.block_on(async { llm.prompt(system, &user).await });
                match response {
                    Ok(text) => {
                        let cleaned = text
                            .trim_start_matches("```json")
                            .trim_start_matches("```")
                            .trim_end_matches("```")
                            .trim();
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(cleaned) {
                            let scores: Vec<f64> = parsed["scores"]
                                .as_array()
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|v| v["score"].as_f64())
                                        .collect()
                                })
                                .unwrap_or_default();
                            if !scores.is_empty() {
                                return scores.iter().sum::<f64>() / scores.len() as f64;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("LLM 权威性评估失败，回退关键词: {e}");
                    }
                }
            }
            Err(_) => {
                tracing::debug!("无 tokio runtime，跳过 LLM 权威性评估");
            }
        }

        self.keyword_authority(urls)
    }

    /// 关键词规则回退
    fn keyword_authority(&self, urls: &[&str]) -> f64 {
        let mut total = 0.0f64;
        for url in urls {
            total += self.score_domain(url);
        }
        total / urls.len() as f64
    }

    fn score_domain(&self, url: &str) -> f64 {
        let domain = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("")
            .trim_start_matches("www.");
        let lower = domain.to_lowercase();

        if lower.ends_with(".edu") || lower.ends_with(".gov") || lower.ends_with(".ac.") {
            return 1.0;
        }
        if lower.contains("arxiv")
            || lower.contains("ieee")
            || lower.contains("acm")
            || lower.contains("springer")
            || lower.contains("elsevier")
            || lower.contains("sciencedirect")
            || lower.contains("jstor")
            || lower.contains("pubmed")
            || lower.contains("dblp")
        {
            return 0.95;
        }
        if lower.contains("github")
            || lower.contains("rust-lang")
            || lower.contains("python")
            || lower.contains("mozilla")
            || lower.contains("w3c")
            || lower.contains("mdn")
            || lower.ends_with(".org")
        {
            return 0.75;
        }
        if lower.contains("medium")
            || lower.contains("stackoverflow")
            || lower.contains("stackexchange")
            || lower.contains("reddit")
            || lower.contains("infoq")
            || lower.contains("oreilly")
        {
            return 0.55;
        }
        0.3
    }

    /// 2. 内部一致性：同源声明是否自洽
    fn dim_consistency(&self, knowledge: &KnowledgeBase) -> f64 {
        // 构建 来源→[发现内容] 映射
        let mut source_findings: HashMap<&str, Vec<&str>> = HashMap::new();
        for f in &knowledge.findings {
            for s in &f.sources {
                source_findings.entry(s.as_str()).or_default().push(&f.content);
            }
        }

        if source_findings.is_empty() {
            return 0.5;
        }

        let mut total_pairs = 0usize;
        let mut low_sim_pairs = 0usize; // Jaccard 相似度 < 0.2 → 可能矛盾

        for (_source, contents) in &source_findings {
            for i in 0..contents.len() {
                for j in (i + 1)..contents.len() {
                    total_pairs += 1;
                    let sim = text_jaccard(contents[i], contents[j]);
                    if sim < 0.2 {
                        low_sim_pairs += 1;
                    }
                }
            }
        }

        if total_pairs == 0 {
            0.5
        } else {
            1.0 - (low_sim_pairs as f64 / total_pairs as f64)
        }
    }

    /// 5. 语义一致性：同子任务的发现是否指向相同结论
    fn dim_semantic(&self, knowledge: &KnowledgeBase) -> f64 {
        // 按 sub_task_id 分组
        let mut groups: HashMap<Option<zhiyuan_core::Uuid>, Vec<&str>> = HashMap::new();
        for f in &knowledge.findings {
            groups.entry(f.sub_task_id).or_default().push(&f.content);
        }

        if groups.len() < 2 {
            return 0.5;
        }

        let mut total_cross = 0usize;
        let mut high_sim_cross = 0usize;

        // 跨不同 sub_task 比较
        let keys: Vec<Option<zhiyuan_core::Uuid>> = groups.keys().cloned().collect();
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                let group_i = &groups[&keys[i]];
                let group_j = &groups[&keys[j]];
                for a in group_i {
                    for b in group_j {
                        total_cross += 1;
                        if text_jaccard(a, b) > 0.3 {
                            high_sim_cross += 1;
                        }
                    }
                }
            }
        }

        if total_cross == 0 {
            0.5
        } else {
            high_sim_cross as f64 / total_cross as f64
        }
    }

    /// 子任务覆盖率：用关键词匹配 findings 覆盖了多少子任务
    fn calc_coverage(&self, knowledge: &KnowledgeBase, plan: &ResearchPlan, query: &str) -> f64 {
        let all_content: String = knowledge
            .findings
            .iter()
            .map(|f| f.content.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");

        if !plan.sub_tasks.is_empty() {
            let covered = plan
                .sub_tasks
                .iter()
                .filter(|st| {
                    let frags = extract_fragments(&st.description);
                    frags.iter().any(|f| all_content.contains(f.as_str()))
                })
                .count();
            covered as f64 / plan.sub_tasks.len() as f64
        } else {
            // 降级：查询关键词匹配
            let frags = extract_fragments(query);
            if frags.is_empty() {
                return 0.5;
            }
            let covered = frags
                .iter()
                .filter(|f| all_content.contains(f.as_str()))
                .count();
            covered as f64 / frags.len() as f64
        }
    }

    /// 多样性：最新一轮迭代的发现占比
    fn calc_freshness(&self, knowledge: &KnowledgeBase) -> f64 {
        if knowledge.findings.is_empty() {
            return 0.0;
        }
        if knowledge.findings.len() <= 1 {
            return 0.5;
        }

        let max_iter = knowledge
            .findings
            .iter()
            .map(|f| f.iteration)
            .max()
            .unwrap_or(1)
            .max(1);
        let threshold = if max_iter > 2 { max_iter - 1 } else { 0 };
        let recent = knowledge
            .findings
            .iter()
            .filter(|f| f.iteration >= threshold)
            .count();
        recent as f64 / knowledge.findings.len() as f64
    }

    /// 深度：技术细节检测（数字、百分比、技术词汇）
    fn calc_depth(&self, knowledge: &KnowledgeBase) -> f64 {
        if knowledge.findings.is_empty() {
            return 0.0;
        }

        let tech_terms = [
            "版本", "标准", "规范", "架构", "协议", "接口", "平台", "API", "SDK", "协议", "框架",
            "引擎", "模块", "组件", "配置", "部署", "集成", "测试", "认证",
        ];

        let mut total = 0.0f64;
        for f in &knowledge.findings {
            let mut s = 0.0f64;
            if f.content.chars().any(|c| c.is_ascii_digit()) {
                s += 0.3;
            }
            if f.content.contains('%') {
                s += 0.2;
            }
            if f.content.len() > 200 {
                s += 0.2;
            }
            if tech_terms.iter().any(|t| f.content.contains(t)) {
                s += 0.3;
            }
            total += s.min(1.0);
        }
        (total / knowledge.findings.len() as f64).min(1.0)
    }
}

impl Default for QualityEvaluator {
    fn default() -> Self {
        Self { llm: None }
    }
}

/// 词级 Jaccard 相似度
fn text_jaccard(a: &str, b: &str) -> f64 {
    let tokenize = |s: &str| -> HashSet<String> {
        s.to_lowercase()
            .split_whitespace()
            .map(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
            })
            .filter(|w| !w.is_empty() && w.len() > 1)
            .map(|w| w.to_string())
            .collect()
    };
    let words_a = tokenize(a);
    let words_b = tokenize(b);
    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// 从文本中提取匹配片段（复制自 extractor.rs 的逻辑，避免跨 crate 依赖）
fn extract_fragments(context: &str) -> Vec<String> {
    let mut frags = Vec::new();

    let raw: Vec<String> = context
        .split(|c: char| {
            !c.is_alphanumeric()
                && !(c as u32 >= 0x4E00 && c as u32 <= 0x9FFF)
                && !(c as u32 >= 0x3400 && c as u32 <= 0x4DBF)
        })
        .filter(|s| s.len() >= 2)
        .flat_map(|s| {
            let mut parts = Vec::new();
            let mut buf = String::new();
            let mut is_cjk = false;
            for c in s.chars() {
                let cur_cjk = (c as u32 >= 0x4E00 && c as u32 <= 0x9FFF)
                    || (c as u32 >= 0x3400 && c as u32 <= 0x4DBF);
                if !buf.is_empty() && cur_cjk != is_cjk {
                    parts.push(std::mem::take(&mut buf));
                }
                buf.push(c);
                is_cjk = cur_cjk;
            }
            if !buf.is_empty() {
                parts.push(buf);
            }
            parts
        })
        .filter(|s| s.len() >= 2)
        .collect();

    for word in &raw {
        let lower = word.to_lowercase();
        if !frags.contains(&lower) {
            frags.push(lower);
        }
    }

    for word in &raw {
        let chars: Vec<char> = word.chars().collect();
        let is_cjk = chars.iter().any(|c| {
            (*c as u32 >= 0x4E00 && *c as u32 <= 0x9FFF)
                || (*c as u32 >= 0x3400 && *c as u32 <= 0x4DBF)
        });
        if !is_cjk || chars.len() < 3 {
            continue;
        }
        for w in chars.windows(2) {
            let ngram: String = w.iter().collect();
            let lower = ngram.to_lowercase();
            if !frags.contains(&lower) {
                frags.push(lower);
            }
        }
        if chars.len() >= 3 {
            for w in chars.windows(3) {
                let ngram: String = w.iter().collect();
                let lower = ngram.to_lowercase();
                if !frags.contains(&lower) {
                    frags.push(lower);
                }
            }
        }
    }

    frags
}
