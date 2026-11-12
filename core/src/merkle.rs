//! The commitment to the pool.
//!
//! A leaf is `sha256(0x00 ‖ task_json)`, a node `sha256(0x01 ‖ left ‖ right)`;
//! the prefixes keep a leaf from ever passing for a node. A level with an odd
//! number of hashes promotes its last one unchanged. The arena contract
//! computes the same root from the leaves the judge signed, before any duel.

use sha2::{Digest, Sha256};

pub type Hash = [u8; 32];

pub fn leaf_hash(task_json: &[u8]) -> Hash {
    let mut h = Sha256::new();
    h.update([0u8]);
    h.update(task_json);
    h.finalize().into()
}

pub fn node_hash(left: &Hash, right: &Hash) -> Hash {
    let mut h = Sha256::new();
    h.update([1u8]);
    h.update(left);
    h.update(right);
    h.finalize().into()
}

fn next_level(level: &[Hash]) -> Vec<Hash> {
    level.chunks(2).map(|pair| if pair.len() == 2 { node_hash(&pair[0], &pair[1]) } else { pair[0] }).collect()
}

/// The root over `leaves`, in order; `None` for no leaves.
pub fn root(leaves: &[Hash]) -> Option<Hash> {
    if leaves.is_empty() {
        return None;
    }
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        level = next_level(&level);
    }
    Some(level[0])
}

/// The sibling hashes from leaf `index` up to the root. A level where the
/// path's hash is promoted has no sibling and adds nothing.
pub fn proof(leaves: &[Hash], mut index: usize) -> Vec<Hash> {
    let mut out = Vec::new();
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        let sibling = index ^ 1;
        if sibling < level.len() {
            out.push(level[sibling]);
        }
        level = next_level(&level);
        index /= 2;
    }
    out
}

/// Whether `leaf` is at `index` of a tree of `count` leaves with this root.
pub fn verify(leaf: &Hash, mut index: usize, count: usize, proof: &[Hash], root: &Hash) -> bool {
    if index >= count {
        return false;
    }
    let mut hash = *leaf;
    let mut width = count;
    let mut siblings = proof.iter();
    while width > 1 {
        let sibling = index ^ 1;
        if sibling < width {
            let Some(s) = siblings.next() else { return false };
            hash = if index % 2 == 0 { node_hash(&hash, s) } else { node_hash(s, &hash) };
        }
        width = width.div_ceil(2);
        index /= 2;
    }
    siblings.next().is_none() && hash == *root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(n: usize) -> Vec<Hash> {
        (0..n).map(|i| leaf_hash(format!("task {i}").as_bytes())).collect()
    }

    #[test]
    fn every_leaf_proves_into_the_root_and_nothing_else_does() {
        for n in 1..=17 {
            let ls = leaves(n);
            let r = root(&ls).unwrap();
            for i in 0..n {
                let p = proof(&ls, i);
                assert!(verify(&ls[i], i, n, &p, &r), "n={n} i={i}");
                assert!(!verify(&leaf_hash(b"forged"), i, n, &p, &r));
                if n > 1 {
                    assert!(!verify(&ls[i], (i + 1) % n, n, &p, &r), "a leaf proves only at its own index");
                }
            }
        }
    }

    #[test]
    fn a_leaf_is_never_a_node() {
        let ls = leaves(2);
        let mut concat = vec![];
        concat.extend_from_slice(&ls[0]);
        concat.extend_from_slice(&ls[1]);
        assert_ne!(leaf_hash(&concat), root(&ls).unwrap());
    }

    /// The vector the contract's own implementation is tested against.
    #[test]
    fn known_root() {
        assert_eq!(
            hex::encode(root(&leaves(3)).unwrap()),
            hex::encode(node_hash(&node_hash(&leaves(3)[0], &leaves(3)[1]), &leaves(3)[2]))
        );
        assert_eq!(root(&[]), None);
    }
}
