//! Hybrid search combining BM25 and semantic embeddings.
//!
//! Pipeline:
//! 1. Semantic search (embeddings) for broad recall - fixes BM25's synonym blindness
//! 2. BM25 re-ranking of top candidates - protects against junk semantic matches
//! 3. Reciprocal Rank Fusion (RRF) to combine rankings

mod bm25;

pub use bm25::{
    Bm25Index, Bm25Params, Bm25SearchResult, FieldWeights, extract_doc_comments,
    extract_identifiers, extract_string_literals, path_tokens, tokenize,
};

use std::collections::HashMap;

/// Configuration for hybrid search.
#[derive(Debug, Clone)]
pub struct HybridSearchConfig {
    /// Number of candidates to retrieve from semantic search (recall phase).
    pub semantic_top_k: usize,
    /// Number of candidates to retrieve from BM25 (for fusion).
    pub bm25_top_k: usize,
    /// Final number of results to return.
    pub final_top_k: usize,
    /// Weight for semantic score in fusion (0.0 to 1.0).
    pub semantic_weight: f32,
    /// Weight for BM25 score in fusion (0.0 to 1.0).
    pub bm25_weight: f32,
    /// RRF constant (typically 60).
    pub rrf_k: f32,
    /// Whether to use RRF (true) or weighted score combination (false).
    pub use_rrf: bool,
}

impl Default for HybridSearchConfig {
    fn default() -> Self {
        Self {
            semantic_top_k: 50,   // Broad recall
            bm25_top_k: 50,       // Also check BM25 candidates
            final_top_k: 10,      // Return top 10
            semantic_weight: 0.4, // Slightly favor BM25 for code
            bm25_weight: 0.6,
            rrf_k: 60.0,
            use_rrf: true, // RRF typically works better
        }
    }
}

/// Result from hybrid search.
#[derive(Debug, Clone)]
pub struct HybridSearchResult {
    /// Document identifier.
    pub doc_id: u32,
    /// Combined score.
    pub score: f32,
    /// Semantic similarity score (if available).
    pub semantic_score: Option<f32>,
    /// BM25 score (if available).
    pub bm25_score: Option<f32>,
    /// Which retrieval methods found this result.
    pub found_by: FoundBy,
}

/// Tracks which retrieval methods found a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoundBy {
    /// Found by semantic search only.
    SemanticOnly,
    /// Found by BM25 only.
    Bm25Only,
    /// Found by both methods (strongest signal).
    Both,
}

/// Hybrid search engine combining semantic and BM25 search.
pub struct HybridSearch {
    config: HybridSearchConfig,
}

impl HybridSearch {
    pub fn new(config: HybridSearchConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(HybridSearchConfig::default())
    }

    /// Perform hybrid search.
    ///
    /// Pipeline:
    /// 1. Get semantic candidates (embeddings ANN search)
    /// 2. Get BM25 candidates
    /// 3. Fuse rankings using RRF or weighted combination
    pub fn search(
        &self,
        query: &str,
        semantic_results: Vec<(u32, f32)>,
        bm25_results: Vec<(u32, f32)>,
    ) -> Vec<HybridSearchResult> {
        if self.config.use_rrf {
            self.search_rrf(query, semantic_results, bm25_results)
        } else {
            self.search_weighted(query, semantic_results, bm25_results)
        }
    }

    /// Reciprocal Rank Fusion (RRF) combining.
    ///
    /// RRF score = sum(1 / (k + rank_i)) for each ranking
    /// This is robust to different score scales and distributions.
    fn search_rrf(
        &self,
        _query: &str,
        semantic_results: Vec<(u32, f32)>,
        bm25_results: Vec<(u32, f32)>,
    ) -> Vec<HybridSearchResult> {
        let mut scores: HashMap<u32, (f32, Option<f32>, Option<f32>, FoundBy)> = HashMap::new();
        let k = self.config.rrf_k;

        // Add semantic results with RRF scoring
        for (rank, (doc_id, sim_score)) in semantic_results.iter().enumerate() {
            let rrf_score = self.config.semantic_weight / (k + rank as f32 + 1.0);
            scores.insert(
                *doc_id,
                (rrf_score, Some(*sim_score), None, FoundBy::SemanticOnly),
            );
        }

        // Add/merge BM25 results with RRF scoring
        for (rank, (doc_id, bm25_score)) in bm25_results.iter().enumerate() {
            let rrf_score = self.config.bm25_weight / (k + rank as f32 + 1.0);

            scores
                .entry(*doc_id)
                .and_modify(|(score, _sem, bm, found)| {
                    *score += rrf_score;
                    *bm = Some(*bm25_score);
                    *found = FoundBy::Both;
                })
                .or_insert((rrf_score, None, Some(*bm25_score), FoundBy::Bm25Only));
        }

        // Convert to results and sort by combined score
        let mut results: Vec<HybridSearchResult> = scores
            .into_iter()
            .map(
                |(doc_id, (score, semantic_score, bm25_score, found_by))| HybridSearchResult {
                    doc_id,
                    score,
                    semantic_score,
                    bm25_score,
                    found_by,
                },
            )
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(self.config.final_top_k);
        results
    }

    /// Weighted score combination.
    ///
    /// Normalizes scores to [0,1] range and combines with weights.
    fn search_weighted(
        &self,
        _query: &str,
        semantic_results: Vec<(u32, f32)>,
        bm25_results: Vec<(u32, f32)>,
    ) -> Vec<HybridSearchResult> {
        let mut scores: HashMap<u32, (f32, Option<f32>, Option<f32>, FoundBy)> = HashMap::new();

        // Normalize semantic scores (already 0-1 for cosine similarity)
        for (doc_id, sim_score) in semantic_results {
            let weighted = self.config.semantic_weight * sim_score;
            scores.insert(
                doc_id,
                (weighted, Some(sim_score), None, FoundBy::SemanticOnly),
            );
        }

        // Normalize and add BM25 scores
        let max_bm25 = bm25_results
            .iter()
            .map(|(_, s)| *s)
            .fold(0.0f32, |a, b| a.max(b));

        for (doc_id, bm25_score) in bm25_results {
            let normalized = if max_bm25 > 0.0 {
                bm25_score / max_bm25
            } else {
                0.0
            };
            let weighted = self.config.bm25_weight * normalized;

            scores
                .entry(doc_id)
                .and_modify(|(score, _sem, bm, found)| {
                    *score += weighted;
                    *bm = Some(bm25_score);
                    *found = FoundBy::Both;
                })
                .or_insert((weighted, None, Some(bm25_score), FoundBy::Bm25Only));
        }

        // Convert to results and sort
        let mut results: Vec<HybridSearchResult> = scores
            .into_iter()
            .map(
                |(doc_id, (score, semantic_score, bm25_score, found_by))| HybridSearchResult {
                    doc_id,
                    score,
                    semantic_score,
                    bm25_score,
                    found_by,
                },
            )
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(self.config.final_top_k);
        results
    }
}

/// Quality metrics for search evaluation.
#[derive(Debug, Clone, Default)]
pub struct SearchQualityMetrics {
    /// Precision at K (how many of top-K are relevant).
    pub precision_at_k: f32,
    /// Recall (how many relevant items were found).
    pub recall: f32,
    /// Mean Reciprocal Rank (1/rank of first relevant result).
    pub mrr: f32,
    /// Normalized Discounted Cumulative Gain.
    pub ndcg: f32,
    /// Number of results found by both methods (strongest signal).
    pub both_count: usize,
    /// Number of results found by semantic only.
    pub semantic_only_count: usize,
    /// Number of results found by BM25 only.
    pub bm25_only_count: usize,
}

impl SearchQualityMetrics {
    /// Calculate metrics given results and ground truth relevance.
    pub fn calculate(results: &[HybridSearchResult], relevant: &[u32], k: usize) -> Self {
        let relevant_set: std::collections::HashSet<_> = relevant.iter().copied().collect();
        let top_k = &results[..results.len().min(k)];

        // Precision@K
        let relevant_in_top_k = top_k
            .iter()
            .filter(|r| relevant_set.contains(&r.doc_id))
            .count();
        let precision_at_k = relevant_in_top_k as f32 / k as f32;

        // Recall
        let found_relevant = results
            .iter()
            .filter(|r| relevant_set.contains(&r.doc_id))
            .count();
        let recall = if relevant.is_empty() {
            0.0
        } else {
            found_relevant as f32 / relevant.len() as f32
        };

        // MRR (reciprocal rank of first relevant result)
        let mrr = results
            .iter()
            .enumerate()
            .find(|(_, r)| relevant_set.contains(&r.doc_id))
            .map(|(i, _)| 1.0 / (i + 1) as f32)
            .unwrap_or(0.0);

        // NDCG
        let dcg: f32 = results
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let rel = if relevant_set.contains(&r.doc_id) {
                    1.0
                } else {
                    0.0
                };
                rel / ((i + 2) as f32).log2()
            })
            .sum();

        let ideal_dcg: f32 = (0..relevant.len().min(results.len()))
            .map(|i| 1.0 / ((i + 2) as f32).log2())
            .sum();

        let ndcg = if ideal_dcg > 0.0 {
            dcg / ideal_dcg
        } else {
            0.0
        };

        // Count by retrieval method
        let both_count = results
            .iter()
            .filter(|r| r.found_by == FoundBy::Both)
            .count();
        let semantic_only_count = results
            .iter()
            .filter(|r| r.found_by == FoundBy::SemanticOnly)
            .count();
        let bm25_only_count = results
            .iter()
            .filter(|r| r.found_by == FoundBy::Bm25Only)
            .count();

        Self {
            precision_at_k,
            recall,
            mrr,
            ndcg,
            both_count,
            semantic_only_count,
            bm25_only_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rrf_basic() {
        let config = HybridSearchConfig::default();
        let search = HybridSearch::new(config);

        // Semantic: A > B > C
        let semantic = vec![(1u32, 0.9), (2u32, 0.7), (3u32, 0.5)];

        // BM25: B > A > C (different ranking)
        let bm25 = vec![(2u32, 5.0), (1u32, 4.0), (3u32, 2.0)];

        let results = search.search("test", semantic, bm25);

        // Both A and B should be found by both methods
        assert!(!results.is_empty());
        // Items found by both methods should rank higher
        let found_by_both: Vec<_> = results
            .iter()
            .filter(|r| r.found_by == FoundBy::Both)
            .collect();
        assert!(found_by_both.len() >= 2);
    }

    #[test]
    fn test_weighted_combination() {
        let config = HybridSearchConfig {
            use_rrf: false,
            semantic_weight: 0.5,
            bm25_weight: 0.5,
            ..Default::default()
        };
        let search = HybridSearch::new(config);

        let semantic = vec![(1u32, 0.8)];
        let bm25 = vec![(1u32, 10.0)];

        let results = search.search("test", semantic, bm25);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].found_by, FoundBy::Both);
        // Score should be combination of both
        assert!(results[0].score > 0.5);
    }
}
