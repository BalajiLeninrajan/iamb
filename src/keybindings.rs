//! # Default Keybindings
//!
//! The keybindings are set up here. We define some iamb-specific keybindings, but the default Vim
//! keys come from [modalkit::env::vim::keybindings].
use std::sync::Mutex;

use modalkit::actions::MacroAction;
use modalkit::editing::cursor::CursorStyle;
use modalkit::env::CommonKeyClass;
use modalkit::env::vim::VimMode;
use modalkit::env::vim::keybindings::{InputStep, VimBindings};
use modalkit::keybindings::dialog::Dialog;
use modalkit::keybindings::{BindingMachine, EdgeEvent, EdgeRepeat, InputBindings};

use crate::base::{Keybindings, MATRIX_ID_WORD, ProgramAction, ProgramContext};
use crate::config::{Keys, VimModes};
use crate::prelude::*;

pub type IambStep = InputStep<IambInfo>;

fn once(key: &TerminalKey) -> (EdgeRepeat, EdgeEvent<TerminalKey, CommonKeyClass>) {
    (EdgeRepeat::Once, EdgeEvent::Key(*key))
}

/// Initialize the default keybinding state.
pub fn setup_keybindings(tunables: &TunableValues) -> Keybindings {
    let mut ism = Keybindings::empty();

    let vim = VimBindings::default()
        .default_split(tunables.default_split.into())
        .submit_on_enter(!tunables.send_on_enter)
        .cursor_open(MATRIX_ID_WORD.clone());

    vim.setup(&mut ism);

    let ctrl_w = "<C-W>".parse::<TerminalKey>().unwrap();
    let ctrl_m = "<C-M>".parse::<TerminalKey>().unwrap();
    let ctrl_z = "<C-Z>".parse::<TerminalKey>().unwrap();
    let key_m_lc = "m".parse::<TerminalKey>().unwrap();
    let key_z_lc = "z".parse::<TerminalKey>().unwrap();
    let shift_enter = "<S-Enter>".parse::<TerminalKey>().unwrap();

    let cwz = vec![once(&ctrl_w), once(&key_z_lc)];
    let cwcz = vec![once(&ctrl_w), once(&ctrl_z)];
    let zoom = IambStep::new()
        .actions(vec![WindowAction::ZoomToggle.into()])
        .goto(VimMode::Normal);

    ism.add_mapping(VimMode::Normal, &cwz, &zoom);
    ism.add_mapping(VimMode::Visual, &cwz, &zoom);
    ism.add_mapping(VimMode::Normal, &cwcz, &zoom);
    ism.add_mapping(VimMode::Visual, &cwcz, &zoom);

    let cwm = vec![once(&ctrl_w), once(&key_m_lc)];
    let cwcm = vec![once(&ctrl_w), once(&ctrl_m)];
    let stoggle = IambStep::new()
        .actions(vec![IambAction::ToggleScrollbackFocus.into()])
        .goto(VimMode::Normal);
    ism.add_mapping(VimMode::Normal, &cwm, &stoggle);
    ism.add_mapping(VimMode::Visual, &cwm, &stoggle);
    ism.add_mapping(VimMode::Normal, &cwcm, &stoggle);
    ism.add_mapping(VimMode::Visual, &cwcm, &stoggle);

    let shift_enter = vec![once(&shift_enter)];
    let newline = IambStep::new().actions(vec![
        InsertTextAction::Type(Char::Single('\n').into(), MoveDir1D::Previous, 1.into()).into(),
    ]);
    ism.add_mapping(VimMode::Insert, &cwm, &newline);
    ism.add_mapping(VimMode::Insert, &shift_enter, &newline);

    ism
}

impl InputBindings<TerminalKey, IambStep> for ApplicationSettings {
    fn setup(&self, bindings: &mut Keybindings) {
        for (modes, keys) in &self.macros {
            for (Keys(input, _), Keys(_, run)) in keys {
                let act = MacroAction::Run(run.clone(), Count::Contextual);
                let step = IambStep::new().actions(vec![act.into()]);
                let input = input.iter().map(once).collect::<Vec<_>>();

                for mode in &modes.0 {
                    bindings.add_mapping(*mode, &input, &step);
                }
            }
        }
    }
}

/// One row of the key hint menu: the next key to press, and what pressing it runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyHint {
    pub key: String,
    pub runs: String,
}

/// The keys typed so far of an unfinished macro, and the keys that can follow them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PendingKeys {
    pub typed: String,
    pub hints: Vec<KeyHint>,
}

impl PendingKeys {
    /// The rows of the menu, with the keys padded into a column.
    pub fn lines(&self) -> Vec<Line<'static>> {
        let pad = self.hints.iter().map(|h| h.key.chars().count()).max().unwrap_or(0);
        let bold = Style::default().add_modifier(StyleModifier::BOLD);

        self.hints
            .iter()
            .map(|hint| {
                let key = format!("{:<pad$}", hint.key);

                Line::from(vec![
                    Span::styled(key, bold),
                    Span::raw("  "),
                    Span::raw(hint.runs.clone()),
                ])
            })
            .collect()
    }
}

pub type SharedPendingKeys = Arc<Mutex<PendingKeys>>;

type MacroList = Vec<(Vec<TerminalKey>, String)>;

/// Show ":spaces<Enter>" as ":spaces". Other macros are shown as written.
fn describe_macro(run: &str) -> &str {
    if run.starts_with(':') {
        run.strip_suffix("<Enter>").unwrap_or(run)
    } else {
        run
    }
}

/// List the keys that continue `typed` towards one of `macros`.
fn key_hints(macros: &[(Vec<TerminalKey>, String)], typed: &[TerminalKey]) -> Vec<KeyHint> {
    // Next key -> the macro it completes, and how many macros go through it.
    let mut next = BTreeMap::<String, (Option<&str>, usize)>::new();

    for (keys, run) in macros {
        if keys.len() <= typed.len() || !keys.starts_with(typed) {
            continue;
        }

        let entry = next.entry(keys[typed.len()].to_string()).or_default();
        entry.1 += 1;

        if keys.len() == typed.len() + 1 {
            entry.0 = Some(run.as_str());
        }
    }

    next.into_iter()
        .map(|(key, (run, count))| {
            let runs = match run {
                Some(run) => describe_macro(run).to_string(),
                None => format!("+{count} more"),
            };

            KeyHint { key, runs }
        })
        .collect()
}

/// Wraps the keybindings to track when the user is partway through typing a macro.
///
/// The keybinding machine does not expose which keys it is waiting on, so this follows along
/// using the configured macros. Vim's builtin sequences like "gg" are not listed.
pub struct HintedBindings {
    inner: Keybindings,
    macros: HashMap<VimMode, MacroList>,
    typed: Vec<TerminalKey>,
    pending: SharedPendingKeys,
}

impl HintedBindings {
    pub fn new(
        inner: Keybindings,
        macros: &HashMap<VimModes, HashMap<Keys, Keys>>,
    ) -> (Self, SharedPendingKeys) {
        let mut table = HashMap::<VimMode, MacroList>::new();

        for (modes, keys) in macros {
            for mode in &modes.0 {
                // Typing text should not pop up a menu, so only modes that take commands count.
                if !matches!(mode, VimMode::Normal | VimMode::Visual) {
                    continue;
                }

                let list = table.entry(*mode).or_default();

                for (Keys(input, _), Keys(_, run)) in keys {
                    list.push((input.clone(), run.clone()));
                }
            }
        }

        let pending = SharedPendingKeys::default();
        let bindings = HintedBindings {
            inner,
            macros: table,
            typed: Vec::new(),
            pending: pending.clone(),
        };

        (bindings, pending)
    }

    fn hints(&self, mode: VimMode) -> Vec<KeyHint> {
        match self.macros.get(&mode) {
            Some(macros) if !self.typed.is_empty() => key_hints(macros, &self.typed),
            _ => Vec::new(),
        }
    }

    fn publish(&mut self, mode: VimMode) {
        let mut hints = self.hints(mode);

        if hints.is_empty() && self.typed.len() > 1 {
            // The sequence led nowhere, and the machine retries its last key as a fresh start.
            self.typed = self.typed.split_off(self.typed.len() - 1);
            hints = self.hints(mode);
        }

        if hints.is_empty() {
            self.typed.clear();
        }

        let typed = self.typed.iter().map(ToString::to_string).collect::<String>();

        if let Ok(mut pending) = self.pending.lock() {
            *pending = PendingKeys { typed, hints };
        }
    }
}

impl BindingMachine<TerminalKey, ProgramAction, RepeatType, ProgramContext, CursorStyle>
    for HintedBindings
{
    fn input_key(&mut self, key: TerminalKey) {
        let mode = self.inner.mode();

        self.typed.push(key);
        self.inner.input_key(key);

        if self.inner.mode() != mode {
            // The key finished a binding that changed modes, so nothing is pending.
            self.typed.clear();
        }

        self.publish(mode);
    }

    fn pop(&mut self) -> Option<(ProgramAction, ProgramContext)> {
        self.inner.pop()
    }

    fn context(&mut self) -> ProgramContext {
        self.inner.context()
    }

    fn show_dialog(&mut self, max_rows: usize, max_cols: usize) -> Vec<Cow<'_, str>> {
        self.inner.show_dialog(max_rows, max_cols)
    }

    fn show_mode(&self) -> Option<String> {
        self.inner.show_mode()
    }

    fn reset_mode(&mut self) {
        self.inner.reset_mode();
        self.typed.clear();
        self.publish(VimMode::Normal);
    }

    fn get_cursor_hint(&self) -> CursorStyle {
        self.inner.get_cursor_hint()
    }

    fn repeat(&mut self, sequence: RepeatType, ctx: Option<ProgramContext>) {
        self.inner.repeat(sequence, ctx)
    }

    fn run_dialog(&mut self, dialog: Box<dyn Dialog<ProgramAction>>) {
        self.inner.run_dialog(dialog)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{mock_settings, mock_tunables};

    fn key(s: &str) -> TerminalKey {
        s.parse().unwrap()
    }

    fn mock_macro(input: &[&str], run: &str) -> (Keys, Keys) {
        let input = input.iter().map(|s| key(s)).collect::<Vec<_>>();

        (Keys(input, String::new()), Keys(Vec::new(), run.to_string()))
    }

    fn hint(key: &str, runs: &str) -> KeyHint {
        KeyHint { key: key.into(), runs: runs.into() }
    }

    fn mock_bindings() -> (HintedBindings, SharedPendingKeys) {
        let mut settings = mock_settings();
        let macros = HashMap::from([
            mock_macro(&["g", "r"], ":reply<Enter>"),
            mock_macro(&["g", "l"], ":react "),
            mock_macro(&["<Bslash>", "s"], ":spaces<Enter>"),
            mock_macro(&["<Bslash>", "u", "c"], ":unreads clear<Enter>"),
        ]);
        settings.macros.insert(VimModes(vec![VimMode::Normal]), macros);

        let mut ism = setup_keybindings(&mock_tunables());
        settings.setup(&mut ism);

        HintedBindings::new(ism, &settings.macros)
    }

    #[test]
    fn test_hints_follow_typed_keys() {
        let (mut bindings, pending) = mock_bindings();
        assert_eq!(*pending.lock().unwrap(), PendingKeys::default());

        bindings.input_key(key("g"));
        let shown = pending.lock().unwrap().clone();
        assert_eq!(shown.typed, "g");
        assert_eq!(shown.hints, vec![hint("l", ":react "), hint("r", ":reply")]);

        // Finishing the macro closes the menu.
        bindings.input_key(key("r"));
        assert_eq!(*pending.lock().unwrap(), PendingKeys::default());
    }

    #[test]
    fn test_hints_group_longer_macros() {
        let (mut bindings, pending) = mock_bindings();

        bindings.input_key(key("<Bslash>"));
        let shown = pending.lock().unwrap().clone();
        assert_eq!(shown.hints, vec![hint("s", ":spaces"), hint("u", "+1 more")]);

        bindings.input_key(key("u"));
        let shown = pending.lock().unwrap().clone();
        assert_eq!(shown.hints, vec![hint("c", ":unreads clear")]);
    }

    #[test]
    fn test_hints_restart_after_dead_end() {
        let (mut bindings, pending) = mock_bindings();

        // "g<Bslash>" is not bound, so the machine starts over from the backslash.
        bindings.input_key(key("g"));
        bindings.input_key(key("<Bslash>"));
        let shown = pending.lock().unwrap().clone();
        assert_eq!(shown.hints, vec![hint("s", ":spaces"), hint("u", "+1 more")]);

        bindings.input_key(key("<Esc>"));
        assert_eq!(*pending.lock().unwrap(), PendingKeys::default());
    }

    #[test]
    fn test_hints_hidden_outside_normal_mode() {
        let (mut bindings, pending) = mock_bindings();

        bindings.input_key(key("i"));
        bindings.input_key(key("g"));
        assert_eq!(*pending.lock().unwrap(), PendingKeys::default());
    }
}
