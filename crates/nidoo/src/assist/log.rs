//! The turns kept in memory: the last [`KEEP`], newest last.

use std::collections::VecDeque;

use nidoo_assist::view::AssistTurn;

/// How many turns the memory log keeps.
pub const KEEP: usize = 100;

#[derive(Debug, Default)]
pub struct TurnLog {
    turns: VecDeque<AssistTurn>,
}

impl TurnLog {
    pub fn push(&mut self, turn: AssistTurn) {
        if self.turns.len() == KEEP {
            self.turns.pop_front();
        }
        self.turns.push_back(turn);
    }

    /// Up to `limit` turns, newest first.
    pub fn newest(&self, limit: usize) -> Vec<AssistTurn> {
        self.turns.iter().rev().take(limit).cloned().collect()
    }

    /// Whether the kept turn `id` is flagged as wrongly understood.
    pub fn is_wrong(&self, id: &str) -> bool {
        self.turns.iter().any(|t| t.id == id && t.wrong)
    }

    /// Flags the turn as wrongly understood; false when it is not kept.
    pub fn mark_wrong(&mut self, id: &str) -> bool {
        match self.turns.iter_mut().find(|t| t.id == id) {
            Some(turn) => {
                turn.wrong = true;
                true
            }
            None => false,
        }
    }
}
