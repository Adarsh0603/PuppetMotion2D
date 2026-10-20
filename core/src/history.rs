//! Undo/redo as a stack of snapshots of the editable state.
//!
//! Callers take a snapshot before an edit and hand it to `record` once the
//! edit is done. Snapshots hold only user input (joints, pose, keys), never derived
//! data like masks, so they are small and restoring one is a full recompute.

pub struct History<T> {
    undo: Vec<T>,
    redo: Vec<T>,
    limit: usize,
}

impl<T: Clone + PartialEq> History<T> {
    /// Keeps at most `limit` undo steps; the oldest are dropped first.
    pub fn new(limit: usize) -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), limit }
    }

    /// Record a finished edit. `before` is the state prior to the edit,
    /// `after` the current state; nothing is recorded if they are equal.
    pub fn record(&mut self, before: T, after: &T) {
        if &before == after {
            return;
        }
        self.undo.push(before);
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Step back. Returns the state to restore, or `None` if there is nothing to undo.
    pub fn undo(&mut self, current: T) -> Option<T> {
        let prev = self.undo.pop()?;
        self.redo.push(current);
        Some(prev)
    }

    /// Step forward again after an undo.
    pub fn redo(&mut self, current: T) -> Option<T> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_round_trip() {
        let mut h = History::new(10);
        let mut state = 1;
        for next in [2, 3] {
            let before = state;
            state = next;
            h.record(before, &state);
        }
        state = h.undo(state).unwrap();
        assert_eq!(state, 2);
        state = h.undo(state).unwrap();
        assert_eq!(state, 1);
        assert!(h.undo(state).is_none());
        state = h.redo(state).unwrap();
        assert_eq!(state, 2);
        state = h.redo(state).unwrap();
        assert_eq!(state, 3);
        assert!(!h.can_redo());
    }

    #[test]
    fn new_edit_clears_redo_and_noops_are_skipped() {
        let mut h = History::new(10);
        h.record(1, &2);
        let state = h.undo(2).unwrap();
        assert!(h.can_redo());
        h.record(state, &5);
        assert!(!h.can_redo());
        h.record(5, &5);
        assert_eq!(h.undo(5), Some(1));
        assert!(!h.can_undo());
    }

    #[test]
    fn limit_drops_oldest() {
        let mut h = History::new(2);
        h.record(1, &2);
        h.record(2, &3);
        h.record(3, &4);
        assert_eq!(h.undo(4), Some(3));
        assert_eq!(h.undo(3), Some(2));
        assert!(h.undo(2).is_none());
    }
}
