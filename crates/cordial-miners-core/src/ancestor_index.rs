//! Ancestor index: answers "does `a` observe `b`?" in O(1).
//!
//! [`Blocklace::observe`] materialises the full predecessor closure of one
//! block as a `BTreeSet<BlockIdentity>`. Approval and ratification ask that
//! question for every block in the DAG, so building one set per block costs
//! O(N^2) time and memory per memo, and the ordering code builds a fresh memo
//! for every leader (issue #263: roughly cubic time and quadratic memory).
//!
//! This index stores the same closures once, as a bit matrix (N rows of N
//! bits, about 18 MiB at 12 000 blocks), and is rebuilt only when the
//! blocklace generation changes.
//!
//! The closure semantics intentionally mirror [`Blocklace::observe`]:
//! the block itself is included, unsigned predecessor references resolve only
//! when unique, and the checkpoint boundary block is included but its
//! predecessors are not expanded.

use std::collections::{HashMap, HashSet};

use crate::blocklace::Blocklace;
use crate::types::{BlockIdentity, NodeId};

// BEGIN PURE (no crate types below; unit-tested against a naive reference)

/// Row-major bit matrix. Row `i` is the set of nodes that node `i` observes,
/// including `i` itself.
pub(crate) struct Closure {
    words: usize,
    bits: Vec<u64>,
}

impl Closure {
    /// `preds[i]` lists the nodes `i` points at. When `stop[i]` is true the
    /// closure of `i` is `{i}` and its predecessors are not expanded.
    pub(crate) fn build(preds: &[Vec<usize>], stop: &[bool]) -> Self {
        let n = preds.len();
        let words = n.div_ceil(64).max(1);
        let mut bits = vec![0u64; n * words];

        // 0 = unvisited, 1 = on the stack, 2 = row complete. Iterative so a
        // long history cannot overflow the call stack.
        let mut state = vec![0u8; n];
        let mut stack: Vec<(usize, usize)> = Vec::new();

        for root in 0..n {
            if state[root] != 0 {
                continue;
            }
            state[root] = 1;
            stack.push((root, 0));

            while let Some(top) = stack.last_mut() {
                let node = top.0;
                let children: &[usize] = if stop[node] { &[] } else { &preds[node] };

                if top.1 < children.len() {
                    let child = children[top.1];
                    top.1 += 1;
                    if state[child] == 0 {
                        state[child] = 1;
                        stack.push((child, 0));
                    }
                    // state 1 means a cycle, which a valid blocklace cannot
                    // contain. The edge is skipped instead of looping.
                } else {
                    let row = node * words;
                    bits[row + node / 64] |= 1u64 << (node % 64);
                    for &child in children {
                        if state[child] == 2 && child != node {
                            for w in 0..words {
                                let v = bits[child * words + w];
                                bits[row + w] |= v;
                            }
                        }
                    }
                    state[node] = 2;
                    stack.pop();
                }
            }
        }

        Self { words, bits }
    }

    /// True when `a` observes `b` (`b` is in the closure of `a`).
    pub(crate) fn observes(&self, a: usize, b: usize) -> bool {
        (self.bits[a * self.words + b / 64] >> (b % 64)) & 1 == 1
    }

    fn count(&self, a: usize) -> u32 {
        self.bits[a * self.words..(a + 1) * self.words]
            .iter()
            .map(|w| w.count_ones())
            .sum()
    }

    /// True when every pair in `members` is comparable (one observes the
    /// other). A chain sorts strictly by closure size, so checking each
    /// neighbouring pair is sufficient and necessary.
    pub(crate) fn is_chain(&self, members: &mut [usize]) -> bool {
        members.sort_by_key(|&m| (self.count(m), m));
        members
            .windows(2)
            .all(|pair| pair[0] != pair[1] && self.observes(pair[1], pair[0]))
    }
}

// END PURE

/// Generation-stamped ancestor index over a [`Blocklace`].
pub(crate) struct AncestorIndex {
    generation: u64,
    pos: HashMap<BlockIdentity, usize>,
    closure: Closure,
    /// Creators with at least one pair of mutually incomparable blocks, i.e.
    /// equivocators. For every other creator approval reduces to observation.
    non_chain_creators: HashSet<NodeId>,
}

impl AncestorIndex {
    pub(crate) fn build(blocklace: &Blocklace) -> Self {
        let ids: Vec<&BlockIdentity> = blocklace.blocks.keys().collect();
        let pos: HashMap<BlockIdentity, usize> = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id.clone(), i))
            .collect();

        // Unsigned references resolve by (content hash, creator) only when the
        // match is unique, exactly as `Blocklace::resolve_identity` does.
        let mut by_unsigned: HashMap<([u8; 32], NodeId), Option<usize>> = HashMap::new();
        for (i, &id) in ids.iter().enumerate() {
            by_unsigned
                .entry((id.content_hash, id.creator.clone()))
                .and_modify(|slot| *slot = None)
                .or_insert(Some(i));
        }

        let resolve = |reference: &BlockIdentity| -> Option<usize> {
            if let Some(&i) = pos.get(reference) {
                return Some(i);
            }
            if !reference.signature.is_empty() {
                return None;
            }
            by_unsigned
                .get(&(reference.content_hash, reference.creator.clone()))
                .copied()
                .flatten()
        };

        let mut preds: Vec<Vec<usize>> = Vec::with_capacity(ids.len());
        let mut stop: Vec<bool> = Vec::with_capacity(ids.len());
        for &id in &ids {
            stop.push(blocklace.is_checkpoint_boundary(id));
            preds.push(
                blocklace.blocks[id]
                    .predecessors
                    .iter()
                    .filter_map(&resolve)
                    .collect(),
            );
        }

        let closure = Closure::build(&preds, &stop);

        let mut by_creator: HashMap<&NodeId, Vec<usize>> = HashMap::new();
        for (i, &id) in ids.iter().enumerate() {
            by_creator.entry(&id.creator).or_default().push(i);
        }
        let non_chain_creators = by_creator
            .into_iter()
            .filter(|(_, members)| members.len() > 1)
            .filter_map(|(creator, mut members)| {
                (!closure.is_chain(&mut members)).then(|| creator.clone())
            })
            .collect();

        Self {
            generation: blocklace.generation(),
            pos,
            closure,
            non_chain_creators,
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Index of a stored identity (exact match; callers resolve first).
    pub(crate) fn index_of(&self, id: &BlockIdentity) -> Option<usize> {
        self.pos.get(id).copied()
    }

    pub(crate) fn observes(&self, observer: usize, observed: usize) -> bool {
        self.closure.observes(observer, observed)
    }

    /// True when all blocks of `creator` are pairwise comparable.
    pub(crate) fn creator_is_chain(&self, creator: &NodeId) -> bool {
        !self.non_chain_creators.contains(creator)
    }
}

#[cfg(test)]
mod tests {
    use super::Closure;
    use std::collections::VecDeque;

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn naive_closure(preds: &[Vec<usize>], stop: &[bool], from: usize) -> Vec<bool> {
        let mut seen = vec![false; preds.len()];
        let mut queue = VecDeque::new();
        seen[from] = true;
        queue.push_back(from);
        while let Some(current) = queue.pop_front() {
            if stop[current] {
                continue;
            }
            for &p in &preds[current] {
                if !seen[p] {
                    seen[p] = true;
                    queue.push_back(p);
                }
            }
        }
        seen
    }

    /// Row order is deliberately not topological, and `n` crosses the 64 and
    /// 128 bit word boundaries.
    #[test]
    fn closure_matches_bfs_on_random_dags() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for case in 0..500 {
            let n = 1 + rng.below(150) as usize;
            let mut preds = vec![Vec::new(); n];
            for (i, node_preds) in preds.iter_mut().enumerate().skip(1) {
                for _ in 0..rng.below(5) {
                    node_preds.push(rng.below(i as u64) as usize);
                }
            }
            let mut perm: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                let j = rng.below(i as u64 + 1) as usize;
                perm.swap(i, j);
            }
            let mut shuffled = vec![Vec::new(); n];
            for i in 0..n {
                shuffled[perm[i]] = preds[i].iter().map(|&p| perm[p]).collect();
            }
            let stop: Vec<bool> = (0..n).map(|_| rng.below(12) == 0).collect();
            let closure = Closure::build(&shuffled, &stop);
            for a in 0..n {
                let expected = naive_closure(&shuffled, &stop, a);
                for (b, &want) in expected.iter().enumerate() {
                    assert_eq!(
                        closure.observes(a, b),
                        want,
                        "case {case} n={n} a={a} b={b}"
                    );
                }
            }
        }
    }

    #[test]
    fn is_chain_matches_pairwise_check() {
        let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
        for case in 0..500 {
            let n = 2 + rng.below(80) as usize;
            let mut preds = vec![Vec::new(); n];
            for (i, node_preds) in preds.iter_mut().enumerate().skip(1) {
                for _ in 0..rng.below(4) {
                    node_preds.push(rng.below(i as u64) as usize);
                }
            }
            let stop: Vec<bool> = (0..n).map(|_| rng.below(15) == 0).collect();
            let closure = Closure::build(&preds, &stop);
            let mut members: Vec<usize> = (0..n).filter(|_| rng.below(3) == 0).collect();
            if members.len() < 2 {
                continue;
            }
            let expected = members.iter().all(|&x| {
                members
                    .iter()
                    .all(|&y| x == y || closure.observes(x, y) || closure.observes(y, x))
            });
            assert_eq!(closure.is_chain(&mut members), expected, "case {case}");
        }
    }

    #[test]
    fn long_chain_does_not_overflow_the_stack() {
        let n = 20_000;
        let preds: Vec<Vec<usize>> = (0..n)
            .map(|i| if i == 0 { Vec::new() } else { vec![i - 1] })
            .collect();
        let closure = Closure::build(&preds, &vec![false; n]);
        assert!(closure.observes(n - 1, 0));
        assert!(!closure.observes(0, n - 1));
    }
}
