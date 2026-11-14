//! A small "last N" list, used by echo layers 3 and 4.

use std::collections::VecDeque;

/// How many wire IDs and content hashes are remembered (plan 3.8).
pub const RECENT: usize = 64;

/// The last `cap` distinct values, oldest dropped first.
#[derive(Clone, Debug)]
pub struct Recent<T> {
    cap: usize,
    items: VecDeque<T>,
}

impl<T: PartialEq> Recent<T> {
    /// An empty list that holds at most `cap` values.
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            items: VecDeque::with_capacity(cap),
        }
    }

    /// Is `v` in the list?
    pub fn contains(&self, v: &T) -> bool {
        self.items.contains(v)
    }

    /// Add `v`. If it is already there, it moves to the newest place.
    pub fn push(&mut self, v: T) {
        if let Some(i) = self.items.iter().position(|x| *x == v) {
            self.items.remove(i);
        }
        if self.items.len() == self.cap {
            self.items.pop_front();
        }
        self.items.push_back(v);
    }

    /// How many values are held.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True if empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_n() {
        let mut r = Recent::new(3);
        for i in 0..5 {
            r.push(i);
        }
        assert_eq!(r.len(), 3);
        assert!(!r.contains(&1));
        assert!(r.contains(&2) && r.contains(&4));
        r.push(2);
        r.push(5);
        assert!(r.contains(&2), "a re-pushed value is refreshed");
        assert!(!r.contains(&3));
    }
}
