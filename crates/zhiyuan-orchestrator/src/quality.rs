use zhiyuan_core::{KnowledgeBase, QualityScore, ResearchPlan};

pub struct QualityEvaluator;

impl QualityEvaluator {
    pub fn evaluate(
        &self,
        knowledge: &KnowledgeBase,
        query: &str,
        plan: &ResearchPlan,
    ) -> QualityScore {
        let coverage = self.calc_coverage(knowledge, plan, query);
        let freshness = self.calc_freshness(knowledge);
        let depth = self.calc_depth(knowledge);

        QualityScore::new(coverage, freshness, depth)
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
