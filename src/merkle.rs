use std::collections::HashMap;

/// Practical depth for the sparse Merkle tree (256 bits = 32 bytes of key).
const DEPTH: usize = 256;

fn default_leaf() -> [u8; 32] {
    *blake3::hash(b"empty").as_bytes()
}

fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut input = [0u8; 64];
    input[..32].copy_from_slice(left);
    input[32..].copy_from_slice(right);
    *blake3::hash(&input).as_bytes()
}

/// Get bit `i` of key (0 = MSB).
fn get_bit(key: &[u8; 32], i: usize) -> bool {
    let byte = i / 8;
    let bit = 7 - (i % 8);
    (key[byte] >> bit) & 1 == 1
}

/// Precompute default hashes at each level. Level 0 = leaf, level DEPTH = root of empty tree.
fn default_hashes() -> Vec<[u8; 32]> {
    let mut defaults = vec![[0u8; 32]; DEPTH + 1];
    defaults[0] = default_leaf();
    for i in 1..=DEPTH {
        defaults[i] = hash_pair(&defaults[i - 1], &defaults[i - 1]);
    }
    defaults
}

/// Sparse Merkle Tree with blake3 hashing and 256-bit depth.
pub struct SparseMerkleTree {
    /// Leaf values keyed by the 32-byte key.
    leaves: HashMap<[u8; 32], Vec<u8>>,
    /// Internal node hashes. Key: (depth_from_root, path as key with irrelevant bits zeroed).
    /// We store the hash at each node along paths that have been touched.
    nodes: HashMap<(usize, [u8; 32]), [u8; 32]>,
    defaults: Vec<[u8; 32]>,
    root: [u8; 32],
}

/// Path prefix: zero out bits at positions >= depth_from_root.
fn path_prefix(key: &[u8; 32], depth_from_root: usize) -> [u8; 32] {
    if depth_from_root == 0 {
        return [0u8; 32];
    }
    let mut result = *key;
    // Zero out bits from position `depth_from_root` onward
    for i in depth_from_root..256 {
        let byte = i / 8;
        let bit = 7 - (i % 8);
        result[byte] &= !(1 << bit);
    }
    result
}

#[derive(Clone, Debug)]
pub struct MerkleProof {
    pub siblings: Vec<[u8; 32]>,
    pub key: [u8; 32],
    pub value: Option<Vec<u8>>,
}

impl SparseMerkleTree {
    pub fn new() -> Self {
        let defaults = default_hashes();
        let root = defaults[DEPTH];
        SparseMerkleTree {
            leaves: HashMap::new(),
            nodes: HashMap::new(),
            defaults,
            root,
        }
    }

    pub fn root(&self) -> [u8; 32] {
        self.root
    }

    fn leaf_hash(&self, key: &[u8; 32]) -> [u8; 32] {
        match self.leaves.get(key) {
            Some(val) => *blake3::hash(val).as_bytes(),
            None => self.defaults[0],
        }
    }

    fn node_hash(&self, depth_from_root: usize, key: &[u8; 32]) -> [u8; 32] {
        if depth_from_root == DEPTH {
            return self.leaf_hash(key);
        }
        let prefix = path_prefix(key, depth_from_root);
        match self.nodes.get(&(depth_from_root, prefix)) {
            Some(h) => *h,
            None => self.defaults[DEPTH - depth_from_root],
        }
    }

    /// Recompute hashes from leaf up to root for the given key.
    fn update_path(&mut self, key: &[u8; 32]) {
        let mut current = self.leaf_hash(key);

        for d in (0..DEPTH).rev() {
            // d = depth_from_root of the parent we're computing
            // At depth_from_root d+1, the node is `current`.
            // Its sibling is determined by bit d of the key.
            let bit = get_bit(key, d);
            let mut sibling_key = *key;
            // Flip bit d to get sibling path
            let byte = d / 8;
            let bit_pos = 7 - (d % 8);
            sibling_key[byte] ^= 1 << bit_pos;

            let sibling = self.node_hash(d + 1, &sibling_key);

            let parent = if bit {
                hash_pair(&sibling, &current)
            } else {
                hash_pair(&current, &sibling)
            };

            let prefix = path_prefix(key, d);
            self.nodes.insert((d, prefix), parent);
            current = parent;
        }

        self.root = current;
    }

    pub fn insert(&mut self, key: &[u8; 32], value: &[u8]) -> [u8; 32] {
        self.leaves.insert(*key, value.to_vec());
        self.update_path(key);
        self.root
    }

    pub fn get(&self, key: &[u8; 32]) -> Option<Vec<u8>> {
        self.leaves.get(key).cloned()
    }

    pub fn inclusion_proof(&self, key: &[u8; 32]) -> MerkleProof {
        let mut siblings = Vec::with_capacity(DEPTH);

        for d in (0..DEPTH).rev() {
            let bit = get_bit(key, d);
            let mut sibling_key = *key;
            let byte = d / 8;
            let bit_pos = 7 - (d % 8);
            sibling_key[byte] ^= 1 << bit_pos;

            let sibling = self.node_hash(d + 1, &sibling_key);
            siblings.push(sibling);
        }

        MerkleProof {
            siblings,
            key: *key,
            value: self.leaves.get(key).cloned(),
        }
    }

    pub fn verify_proof(
        root: &[u8; 32],
        key: &[u8; 32],
        value: Option<&[u8]>,
        proof: &MerkleProof,
    ) -> bool {
        if proof.siblings.len() != DEPTH {
            return false;
        }

        let mut current = match value {
            Some(v) => *blake3::hash(v).as_bytes(),
            None => default_leaf(),
        };

        // siblings[0] is at deepest level (bit DEPTH-1), siblings[DEPTH-1] is at top (bit 0)
        for (i, sibling) in proof.siblings.iter().enumerate() {
            let d = DEPTH - 1 - i; // depth_from_root of the bit
            let bit = get_bit(key, d);
            current = if bit {
                hash_pair(sibling, &current)
            } else {
                hash_pair(&current, sibling)
            };
        }

        current == *root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key(b: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = b;
        k
    }

    #[test]
    fn test_empty_tree_root() {
        let t1 = SparseMerkleTree::new();
        let t2 = SparseMerkleTree::new();
        assert_eq!(t1.root(), t2.root());
        // Root should be the default hash at depth DEPTH
        assert_eq!(t1.root(), default_hashes()[DEPTH]);
    }

    #[test]
    fn test_single_insert() {
        let mut tree = SparseMerkleTree::new();
        let empty_root = tree.root();
        let key = test_key(1);
        let new_root = tree.insert(&key, b"hello");
        assert_ne!(empty_root, new_root);
        assert_eq!(tree.get(&key), Some(b"hello".to_vec()));
    }

    #[test]
    fn test_multiple_inserts() {
        let mut tree = SparseMerkleTree::new();
        let k1 = test_key(1);
        let k2 = test_key(2);
        let k3 = test_key(3);
        tree.insert(&k1, b"a");
        tree.insert(&k2, b"b");
        tree.insert(&k3, b"c");
        assert_eq!(tree.get(&k1), Some(b"a".to_vec()));
        assert_eq!(tree.get(&k2), Some(b"b".to_vec()));
        assert_eq!(tree.get(&k3), Some(b"c".to_vec()));
        assert_eq!(tree.get(&test_key(99)), None);
    }

    #[test]
    fn test_insert_overwrite() {
        let mut tree = SparseMerkleTree::new();
        let key = test_key(1);
        tree.insert(&key, b"first");
        tree.insert(&key, b"second");
        assert_eq!(tree.get(&key), Some(b"second".to_vec()));
    }

    #[test]
    fn test_inclusion_proof() {
        let mut tree = SparseMerkleTree::new();
        let key = test_key(42);
        tree.insert(&key, b"value");
        let proof = tree.inclusion_proof(&key);
        assert!(SparseMerkleTree::verify_proof(
            &tree.root(),
            &key,
            Some(b"value"),
            &proof
        ));
    }

    #[test]
    fn test_inclusion_proof_wrong_root() {
        let mut tree = SparseMerkleTree::new();
        let key = test_key(42);
        tree.insert(&key, b"value");
        let proof = tree.inclusion_proof(&key);
        let fake_root = [0xFFu8; 32];
        assert!(!SparseMerkleTree::verify_proof(
            &fake_root,
            &key,
            Some(b"value"),
            &proof
        ));
    }

    #[test]
    fn test_exclusion_proof() {
        let mut tree = SparseMerkleTree::new();
        tree.insert(&test_key(1), b"exists");
        let absent_key = test_key(99);
        let proof = tree.inclusion_proof(&absent_key);
        assert!(proof.value.is_none());
        assert!(SparseMerkleTree::verify_proof(
            &tree.root(),
            &absent_key,
            None,
            &proof
        ));
    }

    #[test]
    fn test_root_determinism() {
        let mut t1 = SparseMerkleTree::new();
        let mut t2 = SparseMerkleTree::new();
        let k1 = test_key(1);
        let k2 = test_key(2);
        t1.insert(&k1, b"a");
        t1.insert(&k2, b"b");
        t2.insert(&k1, b"a");
        t2.insert(&k2, b"b");
        assert_eq!(t1.root(), t2.root());
    }
}
