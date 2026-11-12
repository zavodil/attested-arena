//! The pool's Merkle root, computed on chain from the leaves the judge
//! signed: node = sha256(0x01 ‖ left ‖ right), an odd hash promoted
//! unchanged — the same tree as `arena_core::merkle`, with NEAR's sha256.

use near_sdk::env;

fn node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(65);
    bytes.push(1u8);
    bytes.extend_from_slice(left);
    bytes.extend_from_slice(right);
    env::sha256_array(&bytes)
}

pub fn root(leaves: &[[u8; 32]]) -> Option<[u8; 32]> {
    if leaves.is_empty() {
        return None;
    }
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        level = level.chunks(2).map(|p| if p.len() == 2 { node(&p[0], &p[1]) } else { p[0] }).collect();
    }
    Some(level[0])
}
