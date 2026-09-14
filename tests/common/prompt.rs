use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::Path;

use dotrift::deploy::{ObstructionChoice, Prompter};

/// Scripted obstruction prompter: answers each prompt from a queue, in order.
/// Panics when the queue is exhausted, so a test that prompts more often than
/// scripted fails loudly instead of silently cancelling.
///
/// Collapses the former `QueuePrompter` / `CancellingPrompter` /
/// `PanickingPrompter` trio into one adapter at the `Prompter` seam:
/// `Prompt::once` / `sequence` for scripted choices, `Prompt::cancel` for the
/// cancelling case, `Prompt::never` for the must-never-fire case.
pub struct Prompt {
    choices: RefCell<VecDeque<ObstructionChoice>>,
    calls: Cell<usize>,
    cancel: bool,
    never: bool,
}

impl Prompt {
    pub fn once(choice: ObstructionChoice) -> Self {
        Self::sequence([choice])
    }

    pub fn sequence(choices: impl IntoIterator<Item = ObstructionChoice>) -> Self {
        Self {
            choices: RefCell::new(choices.into_iter().collect()),
            calls: Cell::new(0),
            cancel: false,
            never: false,
        }
    }

    /// Answers every prompt with cancellation.
    pub fn cancel() -> Self {
        Self {
            choices: RefCell::new(VecDeque::new()),
            calls: Cell::new(0),
            cancel: true,
            never: false,
        }
    }

    /// Panics on any prompt. Proves no prompt fires.
    pub fn never() -> Self {
        Self {
            choices: RefCell::new(VecDeque::new()),
            calls: Cell::new(0),
            cancel: false,
            never: true,
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.get()
    }
}

impl Prompter for Prompt {
    fn prompt(
        &self,
        _entry: &dotrift::config::DeploymentEntry,
        _obstruction: &Path,
    ) -> std::result::Result<ObstructionChoice, tui::prompt::PromptError> {
        self.calls.set(self.calls.get() + 1);
        if self.cancel {
            return Err(tui::prompt::PromptError::Cancelled);
        }
        if self.never {
            panic!("obstruction prompt must not fire in this test");
        }
        Ok(self
            .choices
            .borrow_mut()
            .pop_front()
            .expect("obstruction prompt choices exhausted by test"))
    }
}
