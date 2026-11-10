//! # Default Keybindings
//!
//! The keybindings are set up here. We define some iamb-specific keybindings, but the default Vim
//! keys come from [modalkit::env::vim::keybindings].
use modalkit::actions::MacroAction;
use modalkit::env::CommonKeyClass;
use modalkit::env::vim::VimMode;
use modalkit::env::vim::keybindings::{InputStep, VimBindings};
use modalkit::keybindings::{EdgeEvent, EdgeRepeat, InputBindings};

use crate::base::{Keybindings, MATRIX_ID_WORD};
use crate::config::Keys;
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

    // Vim binds <Space> to the same motion as "l", and a key that already has an action runs it
    // at once instead of waiting for more input. Drop the motion so macros can start with <Space>.
    let space = vec![once(&"<Space>".parse::<TerminalKey>().unwrap())];
    let wait = IambStep::new();
    ism.add_mapping(VimMode::Normal, &space, &wait);
    ism.add_mapping(VimMode::Visual, &space, &wait);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::mock_tunables;
    use modalkit::keybindings::BindingMachine;

    #[test]
    fn test_space_prefixes_a_mapping() {
        let mut ism = setup_keybindings(&mock_tunables());
        let space = "<Space>".parse::<TerminalKey>().unwrap();
        let key_s = "s".parse::<TerminalKey>().unwrap();
        let keys = vec![once(&space), once(&key_s)];
        let step = IambStep::new().actions(vec![WindowAction::ZoomToggle.into()]);
        ism.add_mapping(VimMode::Normal, &keys, &step);

        // <Space> alone waits for the next key instead of moving the cursor.
        ism.input_key(space);
        assert!(ism.pop().is_none());

        ism.input_key(key_s);
        let (act, _) = ism.pop().unwrap();
        assert_eq!(act, WindowAction::ZoomToggle.into());
    }
}
