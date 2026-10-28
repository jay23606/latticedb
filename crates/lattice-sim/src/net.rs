//! Simulated message network with drop, duplicate, reorder and partition faults.

use crate::SeededRng;
use lattice_seams::{Net, NodeId};
use std::collections::{BTreeSet, VecDeque};

/// In-memory network. Faults are driven by the seeded generator.
#[derive(Debug)]
pub struct SimNet {
    queues: std::collections::BTreeMap<NodeId, VecDeque<(NodeId, Vec<u8>)>>,
    rng: SeededRng,
    drop_ppm: u32,
    dup_ppm: u32,
    reorder_ppm: u32,
    blocked: BTreeSet<(NodeId, NodeId)>,
}

impl SimNet {
    /// A network with the given fault rates (parts per million per message).
    pub fn new(seed: u64, drop_ppm: u32, dup_ppm: u32, reorder_ppm: u32) -> Self {
        Self {
            queues: Default::default(),
            rng: SeededRng::new(seed),
            drop_ppm,
            dup_ppm,
            reorder_ppm,
            blocked: BTreeSet::new(),
        }
    }

    /// Blocks traffic from `a` to `b` (one direction, so asymmetric partitions are possible).
    pub fn block(&mut self, a: NodeId, b: NodeId) {
        self.blocked.insert((a, b));
    }

    /// Removes every block.
    pub fn heal(&mut self) {
        self.blocked.clear();
    }

    fn deliver(&mut self, from: NodeId, to: NodeId, bytes: Vec<u8>) {
        let q = self.queues.entry(to).or_default();
        if self.reorder_ppm > 0 && self.rng.chance_ppm(self.reorder_ppm) {
            q.push_front((from, bytes));
        } else {
            q.push_back((from, bytes));
        }
    }
}

impl Net for SimNet {
    fn send(&mut self, from: NodeId, to: NodeId, bytes: Vec<u8>) {
        if self.blocked.contains(&(from, to)) || self.rng.chance_ppm(self.drop_ppm) {
            return;
        }
        if self.rng.chance_ppm(self.dup_ppm) {
            self.deliver(from, to, bytes.clone());
        }
        self.deliver(from, to, bytes);
    }

    fn recv(&mut self, node: NodeId) -> Option<(NodeId, Vec<u8>)> {
        self.queues.get_mut(&node).and_then(VecDeque::pop_front)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_network_delivers_in_order() {
        let mut n = SimNet::new(1, 0, 0, 0);
        n.send(1, 2, vec![1]);
        n.send(1, 2, vec![2]);
        assert_eq!(n.recv(2), Some((1, vec![1])));
        assert_eq!(n.recv(2), Some((1, vec![2])));
        assert_eq!(n.recv(2), None);
    }

    #[test]
    fn partitions_are_directional_and_heal() {
        let mut n = SimNet::new(1, 0, 0, 0);
        n.block(1, 2);
        n.send(1, 2, vec![1]);
        n.send(2, 1, vec![9]);
        assert_eq!(n.recv(2), None);
        assert_eq!(n.recv(1), Some((2, vec![9])));
        n.heal();
        n.send(1, 2, vec![1]);
        assert_eq!(n.recv(2), Some((1, vec![1])));
    }

    #[test]
    fn always_drop_and_always_duplicate() {
        let mut d = SimNet::new(1, 1_000_000, 0, 0);
        d.send(1, 2, vec![1]);
        assert_eq!(d.recv(2), None);
        let mut u = SimNet::new(1, 0, 1_000_000, 0);
        u.send(1, 2, vec![1]);
        assert!(u.recv(2).is_some() && u.recv(2).is_some() && u.recv(2).is_none());
    }
}
