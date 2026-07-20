use instant_distance::{Builder, HnswMap, Point, Search};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// HNSW 向量索引
///
/// 使用 instant-distance 的 HnswMap 进行 ANN 近似最近邻检索。
/// 向量须归一化，索引返回的 distance 是欧氏距离。
pub struct VectorIndex {
    /// HNSW 图 + 值
    map: HnswMap<VecPoint, String>,
    /// 搜索缓存（可复用减少分配）
    search: Search,
    /// 向量维度
    dimension: usize,
}

/// instant-distance 的点类型包装
#[derive(Clone)]
struct VecPoint(Vec<f32>);

impl Point for VecPoint {
    fn distance(&self, other: &Self) -> f32 {
        // 欧氏距离
        self.0
            .iter()
            .zip(other.0.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt()
    }
}

/// 序列化元数据（HNSW 图本身不支持直接序列化）
#[derive(Serialize, Deserialize)]
struct IndexSnapshot {
    keys: Vec<String>,
    vectors: Vec<Vec<f32>>,
    dimension: usize,
}

impl VectorIndex {
    /// 创建空索引
    pub fn new(dimension: usize) -> Self {
        Self {
            map: Builder::default().build(vec![], vec![]),
            search: Search::default(),
            dimension,
        }
    }

    /// 向量维度
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// 当前条目数
    pub fn len(&self) -> usize {
        self.map.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.values.is_empty()
    }

    /// 插入一批向量然后重建索引
    ///
    /// 注意：每次调用都会完全重建 HNSW 图。
    /// 对于小规模索引（< 10 万条）可接受。
    /// keys 和 vectors 长度须匹配。
    pub fn rebuild(&mut self, keys: Vec<String>, vectors: Vec<Vec<f32>>) {
        assert_eq!(keys.len(), vectors.len());
        self.dimension = vectors.first().map(|v| v.len()).unwrap_or(self.dimension);

        let points: Vec<VecPoint> = vectors.into_iter().map(VecPoint).collect();
        let builder = Builder::default();
        self.map = builder.build(points, keys);
        self.search = Search::default();
    }

    /// 搜索最相似的 k 个条目
    ///
    /// 返回 `Vec<(key, cosine_similarity)>`，按相似度降序
    pub fn search(&mut self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        if self.is_empty() {
            return vec![];
        }

        let query_point = VecPoint(query.to_vec());
        let raw: Vec<instant_distance::MapItem<'_, VecPoint, String>> = self
            .map
            .search(&query_point, &mut self.search)
            .take(k)
            .collect();

        raw.into_iter()
            .map(|item| {
                let cosine = 1.0 - (item.distance.powi(2) / 2.0);
                (item.value.clone(), cosine.max(0.0).min(1.0))
            })
            .collect()
    }

    /// 序列化到路径
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = IndexSnapshot {
            keys: self.map.values.clone(),
            vectors: self.map.iter().map(|(_, p)| p.0.clone()).collect(),
            dimension: self.dimension,
        };
        let bytes = bincode::serialize(&snapshot)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// 从路径加载
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let bytes = std::fs::read(path)?;
        let snapshot: IndexSnapshot = bincode::deserialize(&bytes)?;

        let points: Vec<VecPoint> = snapshot.vectors.into_iter().map(VecPoint).collect();
        let map = Builder::default().build(points, snapshot.keys);

        Ok(Self {
            map,
            search: Search::default(),
            dimension: snapshot.dimension,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalize(mut v: Vec<f32>) -> Vec<f32> {
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }

    #[test]
    fn test_insert_and_search() {
        let mut idx = VectorIndex::new(4);
        idx.rebuild(
            vec!["a".into(), "b".into(), "c".into()],
            vec![
                normalize(vec![1.0, 0.0, 0.0, 0.0]),
                normalize(vec![0.0, 1.0, 0.0, 0.0]),
                normalize(vec![0.0, 0.0, 0.0, 1.0]),
            ],
        );

        let results = idx.search(&normalize(vec![1.0, 0.1, 0.0, 0.0]), 2);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "a", "最相似的是 a");
    }

    #[test]
    fn test_save_load() {
        let mut idx = VectorIndex::new(2);
        idx.rebuild(
            vec!["x".into(), "y".into()],
            vec![normalize(vec![1.0, 0.0]), normalize(vec![0.0, 1.0])],
        );

        let path = std::env::temp_dir().join("zhiyuan_test_index.bin");
        idx.save(&path).unwrap();

        let mut loaded = VectorIndex::load(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        let results = loaded.search(&normalize(vec![1.0, 0.0]), 1);
        assert_eq!(results[0].0, "x");

        let _ = std::fs::remove_file(&path);
    }
}
