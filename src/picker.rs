//! # Fuzzy picker
//!
//! A popup that lists items and narrows them down as the user types, like fzf. While a picker is
//! open, the main loop sends keys here instead of to the keybindings.
use modalkit::crossterm::event::KeyCode;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::widgets::{Block, Clear, Padding};

use crate::prelude::*;

/// The widest and tallest the popup gets, borders included.
const MAX_WIDTH: u16 = 72;
const MAX_HEIGHT: u16 = 24;

/// One row of a [Picker].
pub struct PickerItem {
    /// Shown before the text. The query is not matched against this.
    pub prefix: Vec<Span<'static>>,

    /// The text the query is matched against, as styled pieces shown one after another.
    pub text: Vec<(String, Style)>,

    /// What picking this row runs.
    pub action: ProgramAction,
}

impl PickerItem {
    fn haystack(&self) -> String {
        self.text.iter().map(|(text, _)| text.as_str()).collect()
    }

    /// Render this row, drawing the graphemes at the `hits` positions in the `accent` style.
    fn line(&self, hits: &[u32], accent: Style) -> Line<'static> {
        let mut spans = self.prefix.clone();
        let mut index = 0u32;

        for (text, style) in &self.text {
            let mut run = String::new();
            let mut run_hit = false;

            for grapheme in text.graphemes(true) {
                let hit = hits.binary_search(&index).is_ok();

                if hit != run_hit && !run.is_empty() {
                    spans.push(styled_run(std::mem::take(&mut run), *style, run_hit, accent));
                }

                run_hit = hit;
                run.push_str(grapheme);
                index += 1;
            }

            if !run.is_empty() {
                spans.push(styled_run(run, *style, run_hit, accent));
            }
        }

        Line::from(spans)
    }
}

fn styled_run(run: String, style: Style, hit: bool, accent: Style) -> Span<'static> {
    let style = if hit { style.patch(accent) } else { style };

    Span::styled(run, style)
}

/// What a key did to a [Picker].
///
/// One of these exists per keypress and is matched on right away, so the size of the largest
/// variant costs nothing worth boxing it for.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Eq, PartialEq)]
pub enum PickerEvent {
    /// The picker stays open.
    Pending,

    /// The user closed the picker without picking anything.
    Cancel,

    /// The user picked a row, and its action should now run.
    Accept(ProgramAction, ProgramContext),
}

/// A list of items filtered by a query the user types.
pub struct Picker {
    title: String,
    items: Vec<PickerItem>,
    haystacks: Vec<String>,
    query: String,

    /// The items matching the query, best first: an index into `items`, and the positions of the
    /// graphemes in its haystack that matched.
    matches: Vec<(usize, Vec<u32>)>,

    /// Index into `matches` of the highlighted row.
    selected: usize,

    /// Index into `matches` of the first row on screen.
    offset: usize,

    /// How many rows fit on screen, as of the last draw.
    rows: usize,

    ctx: ProgramContext,
    matcher: Matcher,
}

impl Picker {
    pub fn new(title: impl Into<String>, items: Vec<PickerItem>, ctx: ProgramContext) -> Self {
        let haystacks = items.iter().map(PickerItem::haystack).collect();
        let mut picker = Picker {
            title: title.into(),
            items,
            haystacks,
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
            offset: 0,
            rows: 1,
            ctx,
            matcher: Matcher::new(Config::DEFAULT),
        };
        picker.refilter();

        picker
    }

    /// Match every item against the query again, and go back to the top of the list.
    fn refilter(&mut self) {
        self.selected = 0;
        self.offset = 0;

        if self.query.trim().is_empty() {
            self.matches = (0..self.items.len()).map(|index| (index, Vec::new())).collect();
            return;
        }

        // This is fzf's syntax: terms separated by spaces must all match, 'term matches exactly,
        // ^term and term$ anchor to the ends, and !term leaves matching items out.
        let pattern = Pattern::parse(&self.query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored = Vec::new();

        for (index, haystack) in self.haystacks.iter().enumerate() {
            let mut hits = Vec::new();
            let haystack = Utf32Str::new(haystack, &mut buf);

            if let Some(score) = pattern.indices(haystack, &mut self.matcher, &mut hits) {
                hits.sort_unstable();
                hits.dedup();
                scored.push((score, index, hits));
            }
        }

        // Equal scores keep the order the items were given in.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.matches = scored.into_iter().map(|(_, index, hits)| (index, hits)).collect();
    }

    fn select_next(&mut self, count: usize) {
        let last = self.matches.len().saturating_sub(1);
        self.selected = self.selected.saturating_add(count).min(last);
    }

    fn select_previous(&mut self, count: usize) {
        self.selected = self.selected.saturating_sub(count);
    }

    /// Add pasted text to the query.
    pub fn paste(&mut self, text: &str) {
        self.query.extend(text.chars().filter(|c| !c.is_control()));
        self.refilter();
    }

    fn delete_word(&mut self) {
        let kept = self.query.trim_end();
        let kept = kept.rfind(char::is_whitespace).map_or(0, |space| space + 1);
        self.query.truncate(kept);
    }

    fn accept(&mut self) -> PickerEvent {
        let Some((index, _)) = self.matches.get(self.selected) else {
            // Nothing matches the query, so there is nothing to pick yet.
            return PickerEvent::Pending;
        };

        // The picker closes after this, so the other items can end up in any order.
        let item = self.items.swap_remove(*index);

        PickerEvent::Accept(item.action, self.ctx.clone())
    }

    /// Handle a key typed while the picker is open.
    pub fn input_key(&mut self, key: &TerminalKey) -> PickerEvent {
        if *key == TerminalKey::from(KeyCode::Down) {
            self.select_next(1);
        } else if *key == TerminalKey::from(KeyCode::Up) {
            self.select_previous(1);
        } else if *key == TerminalKey::from(KeyCode::PageDown) {
            self.select_next(self.rows);
        } else if *key == TerminalKey::from(KeyCode::PageUp) {
            self.select_previous(self.rows);
        } else {
            match key.get_literal_char() {
                Some('\r') => return self.accept(),
                // <Esc> and <C-C>.
                Some('\u{1B}' | '\u{03}') => return PickerEvent::Cancel,
                // <C-N> and <C-J>.
                Some('\u{0E}' | '\u{0A}') => self.select_next(1),
                // <C-P> and <C-K>.
                Some('\u{10}' | '\u{0B}') => self.select_previous(1),
                // <BS> and <C-H>.
                Some('\u{7F}' | '\u{08}') => {
                    if self.query.pop().is_some() {
                        self.refilter();
                    }
                },
                // <C-W>.
                Some('\u{17}') => {
                    self.delete_word();
                    self.refilter();
                },
                // <C-U>.
                Some('\u{15}') => {
                    self.query.clear();
                    self.refilter();
                },
                Some(c) if !c.is_control() => {
                    self.query.push(c);
                    self.refilter();
                },
                _ => {},
            }
        }

        PickerEvent::Pending
    }

    /// Draw the picker over the middle of `area`.
    pub fn draw(&mut self, f: &mut Frame, area: Rect, theme: &ThemeValues) {
        let width = area.width.saturating_sub(4).min(MAX_WIDTH);
        let height = area.height.saturating_sub(4).min(MAX_HEIGHT);

        if width < 10 || height < 4 {
            // The terminal is too small to show a query and a row inside a border.
            return;
        }

        let x = area.x + (area.width - width) / 2;
        let y = area.y + (area.height - height) / 2;
        let popup = Rect::new(x, y, width, height);

        let title = format!(" {} {}/{} ", self.title, self.matches.len(), self.items.len());
        let block = Block::bordered()
            .title(title)
            .padding(Padding::horizontal(1))
            .border_style(theme.windows.border_focused)
            .border_type(theme.windows.border_focused_line)
            .style(theme.default);
        let inner = block.inner(popup);

        // The query takes the first line.
        let rows = inner.height.saturating_sub(1).max(1) as usize;
        self.rows = rows;

        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + rows {
            self.offset = self.selected + 1 - rows;
        }

        let accent = theme.windows.border_focused.add_modifier(StyleModifier::BOLD);
        let cursor = Style::default().add_modifier(StyleModifier::REVERSED);
        let mut lines = vec![Line::from(vec![
            Span::styled("> ", accent),
            Span::raw(self.query.clone()),
            Span::styled(" ", cursor),
        ])];

        for (index, hits) in self.matches.iter().skip(self.offset).take(rows) {
            lines.push(self.items[*index].line(hits, accent));
        }

        f.render_widget(Clear, popup);
        f.render_widget(Paragraph::new(lines).block(block), popup);

        if !self.matches.is_empty() {
            let y = inner.y + 1 + (self.selected - self.offset) as u16;
            let row = Rect::new(inner.x, y, inner.width, 1);

            f.buffer_mut().set_style(row, theme.selected);
        }
    }
}

/// List every emoji, each picked to react to the selected message with it.
pub fn reaction_items(theme: &ThemeValues) -> Vec<PickerItem> {
    emojis::iter()
        .map(|emoji| {
            let react = MessageAction::React(emoji.as_str().to_string(), true);
            let shortcodes = emoji.shortcodes().collect::<Vec<_>>().join(" ");
            let mut text = Vec::new();

            if !shortcodes.is_empty() {
                text.push((format!("{shortcodes}  "), theme.default));
            }

            // The Unicode name, so that "thumbs up" finds what the shortcodes call "+1".
            text.push((emoji.name().to_string(), theme.rooms.labels));

            PickerItem {
                prefix: vec![Span::raw(format!("{} ", emoji.as_str()))],
                text,
                action: IambAction::from(react).into(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> TerminalKey {
        s.parse().unwrap()
    }

    fn mock_item(name: &str) -> PickerItem {
        PickerItem {
            prefix: vec![],
            text: vec![(name.to_string(), Style::default())],
            action: Action::ShowInfoMessage(name.to_string().into()),
        }
    }

    fn mock_picker(names: &[&str]) -> Picker {
        let items = names.iter().map(|name| mock_item(name)).collect();

        Picker::new("Test", items, ProgramContext::default())
    }

    fn type_query(picker: &mut Picker, query: &str) {
        for c in query.chars() {
            let event = picker.input_key(&TerminalKey::from(KeyCode::Char(c)));
            assert_eq!(event, PickerEvent::Pending);
        }
    }

    /// The names of the rows the picker shows, top to bottom.
    fn shown(picker: &Picker) -> Vec<String> {
        picker
            .matches
            .iter()
            .map(|(index, _)| picker.haystacks[*index].clone())
            .collect()
    }

    fn accepted(name: &str) -> PickerEvent {
        let act = Action::ShowInfoMessage(name.to_string().into());

        PickerEvent::Accept(act, ProgramContext::default())
    }

    #[test]
    fn test_empty_query_lists_everything_in_order() {
        let picker = mock_picker(&["zebra", "apple", "mango"]);
        assert_eq!(shown(&picker), vec!["zebra", "apple", "mango"]);
    }

    #[test]
    fn test_query_matches_subsequences() {
        let mut picker = mock_picker(&["general", "random", "engineering", "design"]);

        // "gnr" is not a substring of anything, but its letters appear in order in two names.
        type_query(&mut picker, "gnr");
        assert_eq!(shown(&picker), vec!["general", "engineering"]);
    }

    #[test]
    fn test_query_terms_must_all_match() {
        let mut picker = mock_picker(&["family chat", "work chat", "family photos"]);

        type_query(&mut picker, "chat fam");
        assert_eq!(shown(&picker), vec!["family chat"]);
    }

    #[test]
    fn test_query_ignores_case_until_uppercase_is_typed() {
        let mut picker = mock_picker(&["Rust", "trust"]);

        type_query(&mut picker, "rust");
        assert_eq!(shown(&picker).len(), 2);

        picker.input_key(&key("<C-U>"));
        type_query(&mut picker, "Rust");
        assert_eq!(shown(&picker), vec!["Rust"]);
    }

    #[test]
    fn test_hits_mark_the_matched_letters() {
        let mut picker = mock_picker(&["abc"]);

        type_query(&mut picker, "ac");
        assert_eq!(picker.matches, vec![(0, vec![0, 2])]);

        let accent = Style::default().add_modifier(StyleModifier::BOLD);
        let line = picker.items[0].line(&[0, 2], accent);
        let spans = line
            .spans
            .iter()
            .map(|span| (span.content.as_ref(), span.style == accent))
            .collect::<Vec<_>>();
        assert_eq!(spans, vec![("a", true), ("b", false), ("c", true)]);
    }

    #[test]
    fn test_enter_accepts_the_selected_row() {
        let mut picker = mock_picker(&["one", "two", "three"]);

        picker.input_key(&key("<C-N>"));
        picker.input_key(&key("<Down>"));
        picker.input_key(&key("<C-P>"));
        assert_eq!(picker.input_key(&key("<Enter>")), accepted("two"));
    }

    #[test]
    fn test_selection_stops_at_the_ends() {
        let mut picker = mock_picker(&["one", "two"]);

        picker.input_key(&key("<Up>"));
        assert_eq!(picker.selected, 0);

        picker.input_key(&key("<C-N>"));
        picker.input_key(&key("<C-N>"));
        picker.input_key(&key("<C-N>"));
        assert_eq!(picker.selected, 1);
    }

    #[test]
    fn test_typing_moves_selection_back_to_the_best_match() {
        let mut picker = mock_picker(&["one", "two", "three"]);

        picker.input_key(&key("<C-N>"));
        type_query(&mut picker, "thr");
        assert_eq!(picker.input_key(&key("<Enter>")), accepted("three"));
    }

    #[test]
    fn test_enter_with_no_match_keeps_the_picker_open() {
        let mut picker = mock_picker(&["one"]);

        type_query(&mut picker, "xyz");
        assert_eq!(shown(&picker), Vec::<String>::new());
        assert_eq!(picker.input_key(&key("<Enter>")), PickerEvent::Pending);
    }

    #[test]
    fn test_escape_cancels() {
        let mut picker = mock_picker(&["one"]);

        assert_eq!(picker.input_key(&key("<Esc>")), PickerEvent::Cancel);
        assert_eq!(picker.input_key(&key("<C-C>")), PickerEvent::Cancel);
    }

    #[test]
    fn test_editing_the_query() {
        let mut picker = mock_picker(&["one"]);

        type_query(&mut picker, "family chat");
        picker.input_key(&key("<BS>"));
        assert_eq!(picker.query, "family cha");

        picker.input_key(&key("<C-W>"));
        assert_eq!(picker.query, "family ");

        picker.input_key(&key("<C-W>"));
        assert_eq!(picker.query, "");

        picker.paste("pasted\ntext");
        assert_eq!(picker.query, "pastedtext");

        picker.input_key(&key("<C-U>"));
        assert_eq!(picker.query, "");
        assert_eq!(shown(&picker), vec!["one"]);
    }

    #[test]
    fn test_reactions_match_shortcodes_and_names() {
        let theme = ThemeValues::default();
        let mut picker =
            Picker::new("Reactions", reaction_items(&theme), ProgramContext::default());

        type_query(&mut picker, "thumbsup");
        let react = MessageAction::React("\u{1F44D}".into(), true);
        let act = Action::from(IambAction::from(react));
        assert_eq!(
            picker.input_key(&key("<Enter>")),
            PickerEvent::Accept(act, ProgramContext::default())
        );
    }
}
