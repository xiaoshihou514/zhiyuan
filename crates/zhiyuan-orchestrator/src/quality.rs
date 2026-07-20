use std::collections::{HashMap, HashSet};
use zhiyuan_core::{KnowledgeBase, QualityScore, ResearchPlan};

pub struct QualityEvaluator;

impl QualityEvaluator {
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

    /// 五维可靠性
    ///
    /// 1. 来源权威性：域名级别（.edu/.gov/学术机构 vs 普通站点）
    /// 2. 内部一致性：同源声明是否自洽（Jaccard 相似度检测矛盾）
    /// 3. 时效一致性：不同来源对同一事实的时效描述是否一致
    /// 4. 提取完整性：内容长度 vs 截断阈值
    /// 5. 语义一致性：同子任务的发现是否指向相同结论
    fn calc_reliability(&self, knowledge: &KnowledgeBase) -> f64 {
        if knowledge.findings.is_empty() {
            return 0.0;
        }

        let authority = self.dim_authority(knowledge);
        let consistency = self.dim_consistency(knowledge);
        let temporal = self.dim_temporal(knowledge);
        let completeness = self.dim_completeness(knowledge);
        let semantic = self.dim_semantic(knowledge);

        // 等权平均
        0.2 * authority + 0.2 * consistency + 0.2 * temporal + 0.2 * completeness + 0.2 * semantic
    }

    /// 1. 来源权威性：通过域名判断来源级别
    fn dim_authority(&self, knowledge: &KnowledgeBase) -> f64 {
        let mut total = 0.0f64;
        let mut count = 0usize;
        let mut seen = HashSet::new();

        for f in &knowledge.findings {
            for url in &f.sources {
                if !seen.insert(url.clone()) {
                    continue;
                }
                count += 1;
                total += self.score_domain(url);
            }
        }

        if count == 0 {
            0.3
        } else {
            total / count as f64
        }
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

        // 学术顶级域
        if lower.ends_with(".edu") || lower.ends_with(".gov") || lower.ends_with(".ac.") {
            return 1.0;
        }
        // 学术出版/预印本
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
        // 技术文档/官方
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
        // 知名科技媒体
        if lower.contains("medium")
            || lower.contains("stackoverflow")
            || lower.contains("stackexchange")
            || lower.contains("reddit")
            || lower.contains("infoq")
            || lower.contains("oreilly")
        {
            return 0.55;
        }
        // 普通站点
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

    /// 3. 时效一致性：不同来源的年份描述是否一致
    fn dim_temporal(&self, knowledge: &KnowledgeBase) -> f64 {
        // 提取所有发现中的年份
        let mut year_sets: Vec<HashSet<i32>> = Vec::new();
        for f in &knowledge.findings {
            let years: HashSet<i32> = extract_years(&f.content);
            if !years.is_empty() {
                year_sets.push(years);
            }
        }

        if year_sets.len() < 2 {
            return 0.5; // 不足以比较
        }

        // 检查所有年份集合是否有交集 → 一致
        let mut consistent = 0usize;
        let mut total = 0usize;
        for i in 0..year_sets.len() {
            for j in (i + 1)..year_sets.len() {
                total += 1;
                if year_sets[i].intersection(&year_sets[j]).next().is_some() {
                    consistent += 1;
                }
            }
        }

        consistent as f64 / total as f64
    }

    /// 4. 提取完整性：发现内容的长度反映提取质量
    fn dim_completeness(&self, knowledge: &KnowledgeBase) -> f64 {
        if knowledge.findings.is_empty() {
            return 0.0;
        }
        let total: f64 = knowledge
            .findings
            .iter()
            .map(|f| {
                let len = f.content.len();
                // 200 字以下：很可能截断/提取失败
                // 500 字以上：完整提取
                match len {
                    l if l >= 500 => 1.0,
                    l if l >= 200 => 0.5 + (l - 200) as f64 / 600.0,
                    _ => len as f64 / 200.0,
                }
            })
            .sum();
        (total / knowledge.findings.len() as f64).min(1.0)
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
        Self
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

/// 从文本中提取所有出现的年份（四位数字，1900-2100）
fn extract_years(text: &str) -> HashSet<i32> {
    text.split_whitespace()
        .filter_map(|w| {
            let clean = w.trim_matches(|c: char| !c.is_ascii_digit());
            if clean.len() == 4 {
                clean.parse::<i32>().ok().filter(|y| (1900..=2100).contains(y))
            } else {
                None
            }
        })
        .collect()
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
