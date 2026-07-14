use zhiyuan_core::{KnowledgeBase, QualityScore};

pub struct QualityEvaluator;

impl QualityEvaluator {
    pub fn evaluate(&self, knowledge: &KnowledgeBase, query: &str) -> QualityScore {
        let coverage = self.calc_coverage(knowledge, query);
        let reliability = self.calc_reliability(knowledge);
        let freshness = self.calc_freshness(knowledge);
        let depth = self.calc_depth(knowledge);

        QualityScore::new(coverage, reliability, freshness, depth)
    }

    fn calc_coverage(&self, knowledge: &KnowledgeBase, query: &str) -> f64 {
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .collect();

        if query_words.is_empty() {
            return 0.5;
        }

        let all_content: String = knowledge
            .findings
            .iter()
            .map(|f| f.content.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");

        let covered = query_words
            .iter()
            .filter(|w| all_content.contains(*w))
            .count();

        covered as f64 / query_words.len() as f64
    }

    fn calc_reliability(&self, knowledge: &KnowledgeBase) -> f64 {
        if knowledge.findings.is_empty() {
            return 0.0;
        }

        let total_sources: usize = knowledge.findings.iter().map(|f| f.sources.len()).sum();
        let findings_with_multiple_sources = knowledge
            .findings
            .iter()
            .filter(|f| f.sources.len() >= 2)
            .count();

        if knowledge.findings.is_empty() {
            return 0.0;
        }

        let multi_source_ratio = findings_with_multiple_sources as f64 / knowledge.findings.len() as f64;
        let source_avg = total_sources as f64 / knowledge.findings.len() as f64;

        0.3 * multi_source_ratio + 0.7 * (source_avg / 5.0).min(1.0)
    }

    fn calc_freshness(&self, knowledge: &KnowledgeBase) -> f64 {
        if knowledge.findings.is_empty() {
            return 0.0;
        }

        let avg_sources = knowledge.findings.iter().map(|f| f.sources.len() as f64).sum::<f64>()
            / knowledge.findings.len() as f64;

        (avg_sources / 3.0).min(1.0)
    }

    fn calc_depth(&self, knowledge: &KnowledgeBase) -> f64 {
        if knowledge.findings.is_empty() {
            return 0.0;
        }

        let avg_length = knowledge
            .findings
            .iter()
            .map(|f| f.content.len() as f64)
            .sum::<f64>()
            / knowledge.findings.len() as f64;

        (avg_length / 500.0).min(1.0)
    }
}

impl Default for QualityEvaluator {
    fn default() -> Self {
        Self
    }
}
