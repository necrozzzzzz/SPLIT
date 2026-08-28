use std::sync::Mutex;

use serde::Serialize;

use super::{parser::PositionSnapshot, slots::SlotBank};

const HISTORY_LIMIT: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct SlotAction {
    pub bank: SlotBank,
    pub slot: u8,
    pub before: Option<PositionSnapshot>,
    pub after: Option<PositionSnapshot>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Default)]
struct History {
    undo_stack: Vec<SlotAction>,
    redo_stack: Vec<SlotAction>,
}

impl History {
    fn state(&self) -> HistoryState {
        HistoryState {
            can_undo: !self.undo_stack.is_empty(),
            can_redo: !self.redo_stack.is_empty(),
        }
    }

    fn push_bounded(stack: &mut Vec<SlotAction>, action: SlotAction) {
        if stack.len() == HISTORY_LIMIT {
            stack.remove(0);
        }
        stack.push(action);
    }

    fn record(&mut self, action: SlotAction) -> bool {
        if action.before == action.after {
            return false;
        }
        Self::push_bounded(&mut self.undo_stack, action);
        self.redo_stack.clear();
        true
    }

    fn peek_undo(&self) -> Option<SlotAction> {
        self.undo_stack.last().cloned()
    }

    fn peek_redo(&self) -> Option<SlotAction> {
        self.redo_stack.last().cloned()
    }

    fn complete_undo(&mut self) {
        if let Some(action) = self.undo_stack.pop() {
            Self::push_bounded(&mut self.redo_stack, action);
        }
    }

    fn complete_redo(&mut self) {
        if let Some(action) = self.redo_stack.pop() {
            Self::push_bounded(&mut self.undo_stack, action);
        }
    }
}

static HISTORY: Mutex<History> = Mutex::new(History {
    undo_stack: Vec::new(),
    redo_stack: Vec::new(),
});

pub fn state() -> Result<HistoryState, String> {
    HISTORY
        .lock()
        .map(|history| history.state())
        .map_err(|_| "History lock poisoned".to_string())
}

pub fn record(action: SlotAction) -> Result<(bool, HistoryState), String> {
    let mut history = HISTORY
        .lock()
        .map_err(|_| "History lock poisoned".to_string())?;
    let changed = history.record(action);
    Ok((changed, history.state()))
}

pub fn peek_undo() -> Result<Option<SlotAction>, String> {
    HISTORY
        .lock()
        .map(|history| history.peek_undo())
        .map_err(|_| "History lock poisoned".to_string())
}

pub fn peek_redo() -> Result<Option<SlotAction>, String> {
    HISTORY
        .lock()
        .map(|history| history.peek_redo())
        .map_err(|_| "History lock poisoned".to_string())
}

pub fn complete_undo() -> Result<HistoryState, String> {
    let mut history = HISTORY
        .lock()
        .map_err(|_| "History lock poisoned".to_string())?;
    history.complete_undo();
    Ok(history.state())
}

pub fn complete_redo() -> Result<HistoryState, String> {
    let mut history = HISTORY
        .lock()
        .map_err(|_| "History lock poisoned".to_string())?;
    history.complete_redo();
    Ok(history.state())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(value: f64) -> Option<PositionSnapshot> {
        Some(PositionSnapshot {
            x: value,
            y: value,
            z: value,
            pitch: value,
            yaw: value,
            roll: value,
            camera: None,
        })
    }

    fn action(before: Option<PositionSnapshot>, after: Option<PositionSnapshot>) -> SlotAction {
        SlotAction {
            bank: SlotBank::Preset(1),
            slot: 1,
            before,
            after,
        }
    }

    #[test]
    fn empty_save_undo_redo_restores_expected_values() {
        let mut history = History::default();
        history.record(action(None, position(1.0)));
        assert_eq!(history.peek_undo().unwrap().before, None);
        history.complete_undo();
        assert_eq!(history.peek_redo().unwrap().after, position(1.0));
        history.complete_redo();
        assert_eq!(history.peek_undo().unwrap().after, position(1.0));
    }

    #[test]
    fn overwrite_undo_redo_preserves_both_values() {
        let mut history = History::default();
        history.record(action(position(1.0), position(2.0)));
        assert_eq!(history.peek_undo().unwrap().before, position(1.0));
        history.complete_undo();
        assert_eq!(history.peek_redo().unwrap().after, position(2.0));
    }

    #[test]
    fn new_save_after_undo_clears_redo() {
        let mut history = History::default();
        history.record(action(None, position(1.0)));
        history.complete_undo();
        history.record(action(None, position(3.0)));
        assert!(history.redo_stack.is_empty());
    }

    #[test]
    fn history_is_bounded_to_32_actions() {
        let mut history = History::default();
        for slot in 0..33 {
            history.record(SlotAction {
                bank: SlotBank::Preset(1),
                slot,
                before: None,
                after: position(f64::from(slot)),
            });
        }
        assert_eq!(history.undo_stack.len(), HISTORY_LIMIT);
        assert_eq!(history.undo_stack[0].slot, 1);
    }

    #[test]
    fn identical_values_do_not_create_history() {
        let mut history = History::default();
        assert!(!history.record(action(position(1.0), position(1.0))));
        assert_eq!(
            history.state(),
            HistoryState {
                can_undo: false,
                can_redo: false
            }
        );
    }

    #[test]
    fn action_preserves_preset_slot_and_values() {
        let mut history = History::default();
        history.record(SlotAction {
            bank: SlotBank::Favorites,
            slot: 7,
            before: position(1.0),
            after: position(2.0),
        });
        let saved = history.peek_undo().unwrap();
        assert_eq!((saved.bank, saved.slot), (SlotBank::Favorites, 7));
        assert_eq!(saved.before, position(1.0));
        assert_eq!(saved.after, position(2.0));
    }

    #[test]
    fn favorite_overwrite_undo_redo_keeps_favorite_bank() {
        let mut history = History::default();
        history.record(SlotAction {
            bank: SlotBank::Favorites,
            slot: 1,
            before: position(1.0),
            after: position(2.0),
        });

        let undo = history.peek_undo().unwrap();
        assert_eq!(undo.bank, SlotBank::Favorites);
        assert_eq!(undo.before, position(1.0));
        history.complete_undo();
        let redo = history.peek_redo().unwrap();
        assert_eq!(redo.bank, SlotBank::Favorites);
        assert_eq!(redo.after, position(2.0));
    }

    #[test]
    fn preset_action_remains_identifiable_from_favorite_mode() {
        let mut history = History::default();
        history.record(SlotAction {
            bank: SlotBank::Preset(2),
            slot: 3,
            before: None,
            after: position(3.0),
        });

        assert_eq!(history.peek_undo().unwrap().bank, SlotBank::Preset(2));
    }
}
