use std::path::Path;

/// Minimal block representation for storage.
/// Will be replaced with or converted from the real Block type in task 0.5.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct StorableBlock {
    pub height: u64,
    pub hash: [u8; 32],
    pub prev_hash: [u8; 32],
    pub timestamp: u64,
    pub tx_hashes: Vec<[u8; 32]>,
    pub data: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sled error: {0}")]
    Sled(#[from] sled::Error),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("block not found")]
    BlockNotFound,
    #[error("transaction not found")]
    TransactionNotFound,
}

pub struct Storage {
    #[allow(dead_code)]
    db: sled::Db,
    blocks: sled::Tree,
    block_hashes: sled::Tree,
    state_roots: sled::Tree,
    tx_index: sled::Tree,
    metadata: sled::Tree,
}

fn height_key(height: u64) -> [u8; 8] {
    height.to_be_bytes()
}

fn cbor_serialize<T: serde::Serialize>(val: &T) -> Result<Vec<u8>, StorageError> {
    let mut buf = Vec::new();
    ciborium::into_writer(val, &mut buf).map_err(|e| StorageError::Serialization(e.to_string()))?;
    Ok(buf)
}

fn cbor_deserialize<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, StorageError> {
    ciborium::from_reader(bytes).map_err(|e| StorageError::Serialization(e.to_string()))
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let db = sled::open(path)?;
        let blocks = db.open_tree("blocks")?;
        let block_hashes = db.open_tree("block_hashes")?;
        let state_roots = db.open_tree("state_roots")?;
        let tx_index = db.open_tree("tx_index")?;
        let metadata = db.open_tree("metadata")?;
        Ok(Self { db, blocks, block_hashes, state_roots, tx_index, metadata })
    }

    pub fn store_block(&self, block: &StorableBlock) -> Result<(), StorageError> {
        let key = height_key(block.height);
        let value = cbor_serialize(block)?;
        self.blocks.insert(key, value)?;
        self.block_hashes.insert(&block.hash, &key)?;
        for (i, tx_hash) in block.tx_hashes.iter().enumerate() {
            let mut tx_val = [0u8; 12];
            tx_val[..8].copy_from_slice(&block.height.to_be_bytes());
            tx_val[8..].copy_from_slice(&(i as u32).to_be_bytes());
            self.tx_index.insert(tx_hash, &tx_val)?;
        }
        Ok(())
    }

    pub fn get_block_by_height(&self, height: u64) -> Result<StorableBlock, StorageError> {
        let data = self.blocks.get(height_key(height))?.ok_or(StorageError::BlockNotFound)?;
        cbor_deserialize(&data)
    }

    pub fn get_block_by_hash(&self, hash: &[u8; 32]) -> Result<StorableBlock, StorageError> {
        let height_bytes = self.block_hashes.get(hash)?.ok_or(StorageError::BlockNotFound)?;
        let data = self.blocks.get(&*height_bytes)?.ok_or(StorageError::BlockNotFound)?;
        cbor_deserialize(&data)
    }

    pub fn get_latest_height(&self) -> Result<Option<u64>, StorageError> {
        match self.blocks.last()? {
            Some((key, _)) => {
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&key);
                Ok(Some(u64::from_be_bytes(buf)))
            }
            None => Ok(None),
        }
    }

    pub fn get_tx_location(&self, tx_hash: &[u8; 32]) -> Result<(u64, u32), StorageError> {
        let val = self.tx_index.get(tx_hash)?.ok_or(StorageError::TransactionNotFound)?;
        let mut h = [0u8; 8];
        let mut i = [0u8; 4];
        h.copy_from_slice(&val[..8]);
        i.copy_from_slice(&val[8..12]);
        Ok((u64::from_be_bytes(h), u32::from_be_bytes(i)))
    }

    pub fn store_state_root(&self, height: u64, root: &[u8; 32]) -> Result<(), StorageError> {
        self.state_roots.insert(height_key(height), root)?;
        Ok(())
    }

    pub fn get_state_root(&self, height: u64) -> Result<[u8; 32], StorageError> {
        let val = self.state_roots.get(height_key(height))?.ok_or(StorageError::BlockNotFound)?;
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&val);
        Ok(buf)
    }

    pub fn set_metadata(&self, key: &str, value: &impl serde::Serialize) -> Result<(), StorageError> {
        let data = cbor_serialize(value)?;
        self.metadata.insert(key.as_bytes(), data)?;
        Ok(())
    }

    pub fn get_metadata<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>, StorageError> {
        match self.metadata.get(key.as_bytes())? {
            Some(data) => Ok(Some(cbor_deserialize(&data)?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_block(height: u64) -> StorableBlock {
        StorableBlock {
            height,
            hash: [height as u8; 32],
            prev_hash: [0u8; 32],
            timestamp: 1000 + height,
            tx_hashes: vec![[height as u8 + 100; 32], [height as u8 + 200; 32]],
            data: vec![1, 2, 3],
        }
    }

    #[test]
    fn test_store_and_retrieve_by_height() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        let block = test_block(42);
        s.store_block(&block).unwrap();
        let got = s.get_block_by_height(42).unwrap();
        assert_eq!(got.height, 42);
        assert_eq!(got.hash, block.hash);
    }

    #[test]
    fn test_store_and_retrieve_by_hash() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        let block = test_block(7);
        s.store_block(&block).unwrap();
        let got = s.get_block_by_hash(&block.hash).unwrap();
        assert_eq!(got.height, 7);
    }

    #[test]
    fn test_latest_height() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        assert_eq!(s.get_latest_height().unwrap(), None);
        s.store_block(&test_block(5)).unwrap();
        s.store_block(&test_block(10)).unwrap();
        s.store_block(&test_block(3)).unwrap();
        assert_eq!(s.get_latest_height().unwrap(), Some(10));
    }

    #[test]
    fn test_tx_index() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        let block = test_block(1);
        s.store_block(&block).unwrap();
        let (h, i) = s.get_tx_location(&block.tx_hashes[0]).unwrap();
        assert_eq!(h, 1);
        assert_eq!(i, 0);
        let (h, i) = s.get_tx_location(&block.tx_hashes[1]).unwrap();
        assert_eq!(h, 1);
        assert_eq!(i, 1);
    }

    #[test]
    fn test_state_root() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        let root = [0xAB; 32];
        s.store_state_root(99, &root).unwrap();
        assert_eq!(s.get_state_root(99).unwrap(), root);
    }

    #[test]
    fn test_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        s.set_metadata("chain_id", &"testnet").unwrap();
        let val: Option<String> = s.get_metadata("chain_id").unwrap();
        assert_eq!(val, Some("testnet".to_string()));

        #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
        struct Config { version: u32 }
        s.set_metadata("config", &Config { version: 1 }).unwrap();
        let c: Option<Config> = s.get_metadata("config").unwrap();
        assert_eq!(c, Some(Config { version: 1 }));

        let missing: Option<String> = s.get_metadata("nope").unwrap();
        assert_eq!(missing, None);
    }

    #[test]
    fn test_persistence() {
        let dir = tempfile::tempdir().unwrap();
        {
            let s = Storage::open(dir.path()).unwrap();
            s.store_block(&test_block(50)).unwrap();
        }
        let s = Storage::open(dir.path()).unwrap();
        let got = s.get_block_by_height(50).unwrap();
        assert_eq!(got.height, 50);
    }

    #[test]
    fn test_block_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        assert!(matches!(s.get_block_by_height(999), Err(StorageError::BlockNotFound)));
    }
}
