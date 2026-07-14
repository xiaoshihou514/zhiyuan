use rocksdb::{DB, Options, IteratorMode};

use std::path::Path;
use std::sync::Arc;
use zhiyuan_core::{Error, Finding, Result};

const CF_WORKING: &str = "working";
const CF_EPISODIC: &str = "episodic";
const CF_SEMANTIC: &str = "semantic";

pub struct ZhiyuanMemory {
    db: Arc<DB>,
}

impl ZhiyuanMemory {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cfs = vec![CF_WORKING, CF_EPISODIC, CF_SEMANTIC];

        let db = DB::open_cf(&opts, path, cfs)
            .map_err(|e| Error::Memory(format!("Failed to open RocksDB: {e}")))?;

        Ok(Self { db: Arc::new(db) })
    }
}

pub struct WorkingMemory {
    db: Arc<DB>,
}

impl WorkingMemory {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        let cf = self.db.cf_handle(CF_WORKING)
            .ok_or_else(|| Error::Memory("column family not found".into()))?;
        self.db
            .put_cf(&cf, key, value)
            .map_err(|e| Error::Memory(format!("WorkingMemory write failed: {e}")))?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let cf = self.db.cf_handle(CF_WORKING)
            .ok_or_else(|| Error::Memory("column family not found".into()))?;
        let value = self.db
            .get_cf(&cf, key)
            .map_err(|e| Error::Memory(format!("WorkingMemory read failed: {e}")))?;
        Ok(value.map(|v| String::from_utf8_lossy(&v).to_string()))
    }

    pub fn clear(&self) -> Result<()> {
        let cf = self.db.cf_handle(CF_WORKING)
            .ok_or_else(|| Error::Memory("column family not found".into()))?;
        let iter = self.db.iterator_cf(&cf, IteratorMode::Start);
        let keys: Vec<Vec<u8>> = iter.filter_map(|r| r.ok().map(|(k, _)| k.to_vec())).collect();
        for key in keys {
            self.db
                .delete_cf(&cf, key)
                .map_err(|e| Error::Memory(format!("WorkingMemory clear failed: {e}")))?;
        }
        Ok(())
    }
}

pub struct EpisodicMemory {
    db: Arc<DB>,
}

impl EpisodicMemory {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    pub fn store_iteration(&self, research_id: &str, iteration: usize, finding: &Finding) -> Result<()> {
        let cf = self.db.cf_handle(CF_EPISODIC)
            .ok_or_else(|| Error::Memory("column family not found".into()))?;
        let key = format!("{research_id}:iteration:{iteration}:{}", finding.id);
        let value = serde_json::to_string(finding)?;
        self.db
            .put_cf(&cf, key.as_bytes(), value.as_bytes())
            .map_err(|e| Error::Memory(format!("EpisodicMemory write failed: {e}")))?;
        Ok(())
    }

    pub fn get_iteration(&self, research_id: &str, iteration: usize) -> Result<Vec<Finding>> {
        let cf = self.db.cf_handle(CF_EPISODIC)
            .ok_or_else(|| Error::Memory("column family not found".into()))?;
        let prefix = format!("{research_id}:iteration:{iteration}:");
        let mut findings = Vec::new();
        let iter = self.db.prefix_iterator_cf(&cf, prefix.as_bytes());
        for item in iter {
            let (_, value) = item.map_err(|e| Error::Memory(format!("EpisodicMemory read failed: {e}")))?;
            let finding: Finding = serde_json::from_slice(&value)?;
            findings.push(finding);
        }
        Ok(findings)
    }
}

pub struct SemanticMemory {
    db: Arc<DB>,
}

impl SemanticMemory {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    pub fn store_entity(&self, name: &str, entity_type: &str, relations: &[String]) -> Result<()> {
        let cf = self.db.cf_handle(CF_SEMANTIC)
            .ok_or_else(|| Error::Memory("column family not found".into()))?;
        let value = serde_json::json!({
            "type": entity_type,
            "relations": relations,
        });
        self.db
            .put_cf(&cf, format!("entity:{name}"), value.to_string())
            .map_err(|e| Error::Memory(format!("SemanticMemory write failed: {e}")))?;
        Ok(())
    }

    pub fn get_entity(&self, name: &str) -> Result<Option<serde_json::Value>> {
        let cf = self.db.cf_handle(CF_SEMANTIC)
            .ok_or_else(|| Error::Memory("column family not found".into()))?;
        let value = self.db
            .get_cf(&cf, format!("entity:{name}"))
            .map_err(|e| Error::Memory(format!("SemanticMemory read failed: {e}")))?;
        match value {
            Some(v) => Ok(Some(serde_json::from_slice(&v)?)),
            None => Ok(None),
        }
    }
}

pub struct MemoryManager {
    pub working: WorkingMemory,
    pub episodic: EpisodicMemory,
    pub semantic: SemanticMemory,
}

impl MemoryManager {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let memory = ZhiyuanMemory::open(path)?;
        Ok(Self {
            working: WorkingMemory::new(memory.db.clone()),
            episodic: EpisodicMemory::new(memory.db.clone()),
            semantic: SemanticMemory::new(memory.db),
        })
    }
}
