use std::{
    collections::HashSet,
    fmt::Debug,
    io::{self, IsTerminal, Write},
};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    queue,
    style::{Attribute, Color, SetAttribute, Stylize},
    terminal::{self, ClearType},
};
use strum::IntoEnumIterator;

/// Optional per-variant presentation overrides for [`SelectPrompt`].
pub trait PromptOption {
    /// Returns a replacement label, or `None` to derive it from the variant name.
    fn label(&self) -> Option<&str> {
        None
    }

    /// Returns a replacement hotkey, or `None` to derive it from the variant name.
    fn hotkey(&self) -> Option<char> {
        None
    }
}

/// Configurable colors for the prompt's rendered elements.
///
/// The defaults render the selected option row and the confirmation line in
/// green; everything else is unstyled. Build on the defaults with struct
/// update syntax to change only the elements you care about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptStyle {
    /// Color of the question title line.
    pub question: Color,
    /// Color of the currently selected option row.
    pub selected: Color,
    /// Color of the selected option's marker.
    pub marker_selected: Color,
    /// Color of the unselected options' markers.
    pub marker_unselected: Color,
    /// Color of the confirmation line printed after `Enter`.
    pub done: Color,
    /// Color of the help line.
    pub help: Color,
}

impl Default for PromptStyle {
    fn default() -> Self {
        Self {
            question: Color::Reset,
            selected: Color::Green,
            marker_selected: Color::Green,
            marker_unselected: Color::Reset,
            done: Color::Green,
            help: Color::Reset,
        }
    }
}

/// Errors returned by a prompt before, during, or after terminal interaction.
#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("no options to display")]
    EmptyOptions,
    #[error("prompt hotkey {0} is not ASCII A-Z")]
    InvalidHotkey(char),
    #[error("prompt hotkey {0} is used by two options")]
    DuplicateHotkey(char),
    #[error("prompt cancelled")]
    Cancelled,
    #[error("{0}")]
    Io(#[from] io::Error),
}

struct OptionEntry<E> {
    value: E,
    label: String,
    hotkey: char,
}

type OptionFilter<E> = Box<dyn Fn(&E) -> bool>;

/// An inline, keyboard-driven enum selection prompt.
pub struct SelectPrompt<E> {
    question: String,
    default: Option<E>,
    filter: Option<OptionFilter<E>>,
    style: PromptStyle,
}

impl<E> SelectPrompt<E> {
    /// Creates a prompt without an explicit question or default.
    pub fn new() -> Self {
        Self {
            question: String::new(),
            default: None,
            filter: None,
            style: PromptStyle::default(),
        }
    }

    /// Sets the question shown above the options.
    pub fn question(mut self, question: impl Into<String>) -> Self {
        self.question = question.into();
        self
    }

    /// Sets the option initially selected by the prompt.
    pub fn default(mut self, value: E) -> Self {
        self.default = Some(value);
        self
    }

    /// Filters the enum variants displayed by the prompt.
    pub fn filter(mut self, predicate: impl Fn(&E) -> bool + 'static) -> Self {
        self.filter = Some(Box::new(predicate));
        self
    }

    /// Sets the colors used when rendering the prompt.
    pub fn style(mut self, style: PromptStyle) -> Self {
        self.style = style;
        self
    }
}

impl<E> Default for SelectPrompt<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> SelectPrompt<E>
where
    E: Clone + Debug + Eq + IntoEnumIterator + PromptOption,
{
    /// Runs the prompt and returns the confirmed enum variant.
    ///
    /// # Errors
    ///
    /// - Returns [`PromptError::EmptyOptions`] if every variant was filtered out.
    /// - Returns [`PromptError::InvalidHotkey`] if an option's hotkey is not ASCII A-Z.
    /// - Returns [`PromptError::DuplicateHotkey`] if two options share a hotkey.
    /// - Returns [`PromptError::Cancelled`] if the prompt is cancelled.
    /// - Returns [`PromptError::Io`] on terminal interaction failures.
    pub fn interact(self) -> Result<E, PromptError> {
        let options = make_options(self.filter.as_deref())?;
        let selected = self
            .default
            .as_ref()
            .and_then(|default| options.iter().position(|option| &option.value == default))
            .unwrap_or(0);

        if !io::stdin().is_terminal() {
            let mut options = options;
            return Ok(options.swap_remove(selected).value);
        }

        let unicode = is_unicode();
        let mut stdout = io::stdout();
        terminal::enable_raw_mode()?;
        let _guard = TerminalGuard;
        queue!(stdout, cursor::Hide)?;

        let mut state = SelectionState::new(options, selected);
        let mut rendered_lines = 0;
        loop {
            rendered_lines = render(
                &mut stdout,
                &self.question,
                &self.style,
                &state,
                unicode,
                rendered_lines,
            )?;
            stdout.flush()?;

            let mut cancel = || {
                clear_prompt(&mut stdout, rendered_lines)?;
                stdout.flush()?;
                Err(PromptError::Cancelled)
            };
            match event::read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                        return cancel();
                    }
                    KeyCode::Esc if key.modifiers.is_empty() => return cancel(),
                    KeyCode::Up | KeyCode::Left if key.modifiers.is_empty() => state.previous(),
                    KeyCode::BackTab if key.modifiers == KeyModifiers::SHIFT => state.previous(),
                    KeyCode::Down | KeyCode::Right | KeyCode::Tab if key.modifiers.is_empty() => {
                        state.next()
                    }
                    KeyCode::Char(c) if c.is_ascii_alphabetic() && key.modifiers.is_empty() => {
                        state.select_hotkey(c.to_ascii_lowercase());
                    }
                    KeyCode::Enter if key.modifiers.is_empty() => {
                        clear_prompt(&mut stdout, rendered_lines)?;
                        let option = &state.options[state.selected];
                        let marker = if unicode { "✓" } else { "done" };
                        writeln!(
                            stdout,
                            "{} ({}) {}",
                            self.question,
                            option.label,
                            marker.with(self.style.done)
                        )?;
                        stdout.flush()?;
                        return Ok(option.value.clone());
                    }
                    _ => {}
                },
                Event::Resize(..) => {}
                _ => (),
            }
        }
    }
}

struct SelectionState<E> {
    options: Vec<OptionEntry<E>>,
    selected: usize,
}

impl<E> SelectionState<E> {
    fn new(options: Vec<OptionEntry<E>>, selected: usize) -> Self {
        Self { options, selected }
    }

    fn next(&mut self) {
        self.selected = (self.selected + 1) % self.options.len();
    }

    fn previous(&mut self) {
        self.selected = (self.selected + self.options.len() - 1) % self.options.len();
    }

    fn select_hotkey(&mut self, hotkey: char) {
        if let Some(index) = self
            .options
            .iter()
            .position(|option| option.hotkey == hotkey)
        {
            self.selected = index;
        }
    }
}

fn make_options<E>(filter: Option<&dyn Fn(&E) -> bool>) -> Result<Vec<OptionEntry<E>>, PromptError>
where
    E: Clone + Debug + IntoEnumIterator + PromptOption,
{
    let variants = E::iter()
        .filter(|value| filter.is_none_or(|predicate| predicate(value)))
        .collect::<Vec<_>>();
    if variants.is_empty() {
        return Err(PromptError::EmptyOptions);
    }

    let mut options = Vec::with_capacity(variants.len());
    let mut seen_hotkeys = HashSet::with_capacity(variants.len());
    for value in variants {
        let derived_name = format!("{value:?}");
        let derived_label = pascal_to_label(&derived_name);
        let hotkey = value
            .hotkey()
            .unwrap_or_else(|| first_ascii_letter(&derived_label).unwrap_or(' '));
        let hotkey = validate_hotkey(hotkey)?;
        if !seen_hotkeys.insert(hotkey) {
            return Err(PromptError::DuplicateHotkey(hotkey));
        }
        let label = value.label().map_or(derived_label, ToOwned::to_owned);
        options.push(OptionEntry {
            value,
            label,
            hotkey,
        });
    }
    Ok(options)
}

fn validate_hotkey(hotkey: char) -> Result<char, PromptError> {
    if !hotkey.is_ascii_alphabetic() {
        return Err(PromptError::InvalidHotkey(hotkey));
    }
    Ok(hotkey.to_ascii_lowercase())
}

fn first_ascii_letter(value: &str) -> Option<char> {
    value
        .chars()
        .find(|character| character.is_ascii_alphabetic())
}

fn pascal_to_label(value: &str) -> String {
    let characters: Vec<char> = value.chars().collect();
    let mut label = String::with_capacity(value.len());
    for (index, character) in characters.iter().copied().enumerate() {
        let previous = index.checked_sub(1).and_then(|index| characters.get(index));
        let next = characters.get(index + 1);
        let starts_word = character.is_ascii_uppercase()
            && previous.is_some_and(|previous| previous.is_ascii_lowercase())
            || character.is_ascii_uppercase()
                && previous.is_some_and(|previous| previous.is_ascii_uppercase())
                && next.is_some_and(|next| next.is_ascii_lowercase());
        if starts_word && !label.is_empty() {
            label.push(' ');
        }
        label.push(character.to_ascii_lowercase());
    }
    label
}

fn render<E>(
    stdout: &mut impl Write,
    question: &str,
    style: &PromptStyle,
    state: &SelectionState<E>,
    unicode: bool,
    previous_lines: usize,
) -> io::Result<usize> {
    clear_prompt(stdout, previous_lines)?;
    let (_, rows) = terminal::size()?;
    let visible_count = usize::from(rows).saturating_sub(3).max(1);
    let start = state
        .selected
        .saturating_sub(visible_count.saturating_sub(1))
        .min(state.options.len().saturating_sub(visible_count));
    let end = (start + visible_count).min(state.options.len());
    writeln!(stdout, "{}", question.with(style.question))?;
    queue!(stdout, cursor::MoveToColumn(0))?;
    let (selected, unselected) = if unicode { ('●', '○') } else { ('*', ' ') };
    for (index, option) in state.options[start..end].iter().enumerate() {
        let index = index + start;
        let is_selected = index == state.selected;
        let marker = if is_selected { selected } else { unselected };
        let marker_color = if is_selected {
            style.marker_selected
        } else {
            style.marker_unselected
        };
        let row_color = if is_selected {
            style.selected
        } else {
            Color::Reset
        };
        writeln!(
            stdout,
            "  {}{}",
            marker.with(marker_color),
            format!(" [{}] {}", option.hotkey, option.label).with(row_color)
        )?;
        queue!(stdout, cursor::MoveToColumn(0))?;
    }
    writeln!(
        stdout,
        "{}",
        "\n  ↑/↓ navigate  Enter select  A-Z jump  Esc cancel".with(style.help)
    )?;
    Ok(end - start + 3)
}

fn clear_prompt(stdout: &mut impl Write, lines: usize) -> io::Result<()> {
    if lines > 0 {
        queue!(
            stdout,
            cursor::MoveUp(lines as u16),
            cursor::MoveToColumn(0)
        )?;
    }
    queue!(stdout, terminal::Clear(ClearType::FromCursorDown))
}

fn is_unicode() -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"].iter().any(|variable| {
        std::env::var(variable)
            .map(|value| value.contains("UTF-8") || value.contains("utf8"))
            .unwrap_or(false)
    })
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = queue!(
            stdout,
            cursor::Show,
            cursor::MoveToColumn(0),
            SetAttribute(Attribute::Reset)
        );
        let _ = stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::EnumIter;
    use test_case::test_case;

    #[derive(Clone, Debug, PartialEq, Eq, EnumIter)]
    enum Choice {
        Bike,
        Carpool,
        EVScooter,
        Metro,
        Tram,
    }

    impl super::PromptOption for Choice {
        fn label(&self) -> Option<&str> {
            match self {
                Choice::Tram => Some("custom label"),
                _ => None,
            }
        }

        fn hotkey(&self) -> Option<char> {
            match self {
                Choice::Carpool => Some('z'),
                _ => None,
            }
        }
    }

    #[test_case(Choice::Bike, "bike", 'b'; "derived")]
    #[test_case(Choice::EVScooter, "ev scooter", 'e'; "derived_acronym")]
    #[test_case(Choice::Tram, "custom label", 't'; "label_overridden_hotkey_derived")]
    #[test_case(Choice::Carpool, "carpool", 'z'; "hotkey_overridden_label_derived")]
    fn make_options_produces_labels_and_hotkeys(variant: Choice, label: &str, hotkey: char) {
        let options = make_options::<Choice>(None).unwrap();
        let option = options
            .iter()
            .find(|option| option.value == variant)
            .unwrap();
        assert_eq!(option.label, label);
        assert_eq!(option.hotkey, hotkey);
    }

    #[test]
    fn make_options_filters_variants_before_building_options() {
        let options = make_options::<Choice>(Some(&|choice| *choice != Choice::EVScooter)).unwrap();

        assert_eq!(
            options
                .iter()
                .map(|option| option.value.clone())
                .collect::<Vec<_>>(),
            vec![Choice::Bike, Choice::Carpool, Choice::Metro, Choice::Tram]
        );
    }

    #[test]
    fn make_options_rejects_a_filter_that_removes_every_variant() {
        assert!(matches!(
            make_options::<Choice>(Some(&|_| false)),
            Err(PromptError::EmptyOptions)
        ));
    }

    #[test_case("OverwriteIdentical" => "overwrite identical"; "words")]
    #[test_case("HTTPServer" => "http server"; "acronym_run")]
    fn pascal_to_label_converts(value: &str) -> String {
        pascal_to_label(value)
    }

    #[derive(Clone, Debug, PartialEq, Eq, EnumIter)]
    enum DuplicateHotkeys {
        First,
        Second,
    }

    impl super::PromptOption for DuplicateHotkeys {
        fn hotkey(&self) -> Option<char> {
            Some('a')
        }
    }

    #[test]
    fn make_options_rejects_duplicate_hotkeys() {
        assert!(matches!(
            make_options::<DuplicateHotkeys>(None),
            Err(PromptError::DuplicateHotkey('a'))
        ));
    }

    #[derive(Clone, Debug, PartialEq, Eq, EnumIter)]
    enum InvalidHotkey {
        First,
    }

    impl super::PromptOption for InvalidHotkey {
        fn hotkey(&self) -> Option<char> {
            Some('1')
        }
    }

    #[test]
    fn make_options_rejects_non_ascii_hotkey() {
        assert!(matches!(
            make_options::<InvalidHotkey>(None),
            Err(PromptError::InvalidHotkey('1'))
        ));
    }

    #[test]
    fn selection_previous_wraps_to_last() {
        let mut state = SelectionState {
            options: vec![1, 2, 3]
                .into_iter()
                .map(|value| super::OptionEntry {
                    value,
                    label: value.to_string(),
                    hotkey: char::from_digit(value, 10).unwrap(),
                })
                .collect(),
            selected: 0,
        };

        state.previous();

        assert_eq!(state.selected, 2);
    }

    #[test]
    fn selection_next_wraps_to_first() {
        let mut state = SelectionState {
            options: vec![1, 2, 3]
                .into_iter()
                .map(|value| super::OptionEntry {
                    value,
                    label: value.to_string(),
                    hotkey: char::from_digit(value, 10).unwrap(),
                })
                .collect(),
            selected: 2,
        };

        state.next();

        assert_eq!(state.selected, 0);
    }
}
