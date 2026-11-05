//! # Explorer sidebar
//!
//! The explorer is a narrow list of chats that stays docked on the left side of a tab, like the
//! file explorer in a text editor. Selecting a room opens it in the window to the right of the
//! explorer instead of replacing the explorer itself, and typing into the filter bar at the top
//! narrows the list down by room name.
use modalkit::actions::{EditAction, WindowContainer, WindowCount};
use modalkit::editing::context::EditContext;
use modalkit_ratatui::Window;
use modalkit_ratatui::list::List;
use modalkit_ratatui::screen::ScreenState;
use modalkit_ratatui::textbox::{TextBox, TextBoxState};
use modalkit_ratatui::windows::WindowLayoutState;

use crate::base::ExplorerAction;
use crate::prelude::*;
use crate::windows::{GenericRoomItem, IambWindow, RoomLikeItem, RoomListState, room_fields_cmp};

type IambScreen = ScreenState<IambWindow, IambInfo>;

/// Which part of the explorer currently receives editing commands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExplorerFocus {
    /// The list of chats.
    #[default]
    List,

    /// The filter bar above the list.
    Filter,
}

/// Check whether a room name matches the text typed into the explorer's filter bar.
///
/// Every whitespace-separated word in the filter has to appear somewhere in the name, ignoring
/// case, so "dev chat" matches "Chat for devs". Newlines count as whitespace, since Enter types
/// one into the filter bar when `send_on_enter` is disabled.
pub fn filter_matches(name: &str, filter: &str) -> bool {
    let name = name.to_lowercase();

    filter.split_whitespace().all(|word| name.contains(&word.to_lowercase()))
}

pub struct ExplorerState {
    list: RoomListState,
    filter: TextBoxState<IambInfo>,
    focus: ExplorerFocus,

    /// The filter text used during the previous draw. When it changes, the cursor goes back to
    /// the top of the list so that the best match is selected.
    last_filter: String,
}

impl ExplorerState {
    pub fn new(store: &mut ProgramStore) -> Self {
        let buf = store.load_buffer(IambBufferId::ExplorerFilter);
        let filter = TextBoxState::new(buf);
        let last_filter = filter.get_text();
        let list = RoomListState::new(IambBufferId::Explorer, vec![]);

        ExplorerState {
            list,
            filter,
            focus: ExplorerFocus::List,
            last_filter,
        }
    }

    fn items(&self, store: &mut ProgramStore) -> Vec<GenericRoomItem> {
        let ChatStore {
            collator,
            rooms,
            spaces,
            settings,
            sync_info,
            worker,
            ..
        } = &mut store.application;
        let client = &worker.client;
        let filter = self.filter.get_text();

        let mut items = sync_info
            .rooms
            .iter()
            .chain(sync_info.dms.iter())
            .map(|room| {
                GenericRoomItem::new(
                    room,
                    rooms.get_or_default(room.room_id().to_owned()),
                    client,
                    spaces,
                )
                .show_room_type(false)
            })
            .filter(|item| filter_matches(item.name(), &filter))
            .collect::<Vec<_>>();

        let fields = &settings.tunables.sort.chats;
        items.sort_by(|a, b| room_fields_cmp(a, b, fields, collator));
        items.iter_mut().for_each(|i| i.set_section(fields));

        items
    }

    /// Rebuild the list of chats, keeping the cursor on the same room when the list gets
    /// re-sorted (for example, because a new message arrived in another room).
    fn refresh(&mut self, store: &mut ProgramStore) {
        let filter = self.filter.get_text();
        let filter_changed = filter != self.last_filter;
        let selected = self.list.get().map(|item| item.room_id().to_owned());

        if filter_changed {
            self.list = RoomListState::new(IambBufferId::Explorer, vec![]);
            self.last_filter = filter;
        }

        let items = self.items(store);
        let target = selected
            .as_deref()
            .filter(|_| !filter_changed)
            .and_then(|id| items.iter().position(|item| item.room_id() == id));

        self.list.set(items);
        self.list.set_ignorecase(store.application.settings.tunables.ignorecase);

        let moved = self.list.get().map(|item| item.room_id()) != selected.as_deref();

        if moved && let Some(idx) = target {
            let mov = EditTarget::Motion(MoveType::BufferLineOffset, Count::Exact(idx + 1));
            let act = EditorAction::Edit(Specifier::Exact(EditAction::Motion), mov);
            let _ = self.list.editor_command(&act, &EditContext::default(), store);
        }
    }

    /// Whether editing actions should move focus from the list to the filter bar.
    fn wants_filter(act: &EditorAction, ctx: &ProgramContext) -> bool {
        ctx.get_insert_style().is_some() || matches!(act, EditorAction::InsertText(_))
    }

    /// Whether this is a motion that moves up or down by lines.
    fn is_line_motion(act: &EditorAction, ctx: &ProgramContext) -> bool {
        match act {
            EditorAction::Edit(op, EditTarget::Motion(MoveType::Line(_), _)) => {
                ctx.resolve(op).is_motion()
            },
            _ => false,
        }
    }
}

macro_rules! delegate {
    ($s: expr, $id: ident => $e: expr) => {
        match $s.focus {
            ExplorerFocus::List => {
                let $id = &mut $s.list;
                $e
            },
            ExplorerFocus::Filter => {
                let $id = &mut $s.filter;
                $e
            },
        }
    };
}

macro_rules! delegate_ref {
    ($s: expr, $id: ident => $e: expr) => {
        match $s.focus {
            ExplorerFocus::List => {
                let $id = &$s.list;
                $e
            },
            ExplorerFocus::Filter => {
                let $id = &$s.filter;
                $e
            },
        }
    };
}

impl Editable<ProgramContext, ProgramStore, IambInfo> for ExplorerState {
    fn editor_command(
        &mut self,
        act: &EditorAction,
        ctx: &ProgramContext,
        store: &mut ProgramStore,
    ) -> EditResult<EditInfo, IambInfo> {
        match self.focus {
            ExplorerFocus::List if Self::wants_filter(act, ctx) => {
                // Typing while the list is focused goes into the filter bar.
                self.focus = ExplorerFocus::Filter;
                self.filter.editor_command(act, ctx, store)
            },
            ExplorerFocus::List => self.list.editor_command(act, ctx, store),
            ExplorerFocus::Filter if Self::is_line_motion(act, ctx) => {
                // The filter bar is a single line, so moving up or down moves through the list
                // instead. Outside of Insert mode, this also hands focus back to the list.
                if ctx.get_insert_style().is_none() {
                    self.focus = ExplorerFocus::List;
                }

                self.list.editor_command(act, ctx, store)
            },
            ExplorerFocus::Filter => self.filter.editor_command(act, ctx, store),
        }
    }
}

impl Jumpable<ProgramContext, IambInfo> for ExplorerState {
    fn jump(
        &mut self,
        list: PositionList,
        dir: MoveDir1D,
        count: usize,
        ctx: &ProgramContext,
    ) -> IambResult<usize> {
        delegate!(self, w => w.jump(list, dir, count, ctx))
    }
}

impl Scrollable<ProgramContext, ProgramStore, IambInfo> for ExplorerState {
    fn scroll(
        &mut self,
        style: &ScrollStyle,
        ctx: &ProgramContext,
        store: &mut ProgramStore,
    ) -> EditResult<EditInfo, IambInfo> {
        // There's nothing to scroll in the one-line filter bar, so always scroll the list.
        self.list.scroll(style, ctx, store)
    }
}

impl Promptable<ProgramContext, ProgramStore, IambInfo> for ExplorerState {
    fn prompt(
        &mut self,
        act: &PromptAction,
        ctx: &ProgramContext,
        store: &mut ProgramStore,
    ) -> EditResult<Vec<(ProgramAction, ProgramContext)>, IambInfo> {
        match act {
            PromptAction::Submit => {
                let Some(item) = self.list.get() else {
                    let msg = "No chat selected in the explorer";
                    return Err(EditError::Failure(msg.into()));
                };

                let open = IambAction::from(ExplorerAction::Open(item.room_id().to_owned()));

                Ok(vec![(open.into(), ctx.clone())])
            },
            _ => self.list.prompt(act, ctx, store),
        }
    }
}

impl TerminalCursor for ExplorerState {
    fn get_term_cursor(&self) -> Option<TermOffset> {
        delegate_ref!(self, w => w.get_term_cursor())
    }

    fn hide_term_cursor(&self) -> bool {
        delegate_ref!(self, w => w.hide_term_cursor())
    }
}

impl WindowOps<IambInfo> for ExplorerState {
    fn draw(&mut self, area: Rect, buf: &mut Buffer, focused: bool, store: &mut ProgramStore) {
        self.refresh(store);

        let theme = &store.application.settings.theme;
        let rooms_style = theme.rooms.default;
        let filter_style = theme.default;
        let prompt_style = theme.windows.title;

        let filterh = area.height.min(1);
        let filter_area = Rect::new(area.x, area.y, area.width, filterh);
        let list_area = Rect::new(area.x, area.y + filterh, area.width, area.height - filterh);

        let prompt = Span::styled("/ ", prompt_style);
        self.filter.set_ignorecase(store.application.settings.tunables.ignorecase);
        TextBox::new().oneline().style(filter_style).prompt(prompt).render(
            filter_area,
            buf,
            &mut self.filter,
        );

        let empty = if self.last_filter.trim().is_empty() {
            "You do not have rooms or dms yet"
        } else {
            "No chats match the filter"
        };

        List::new(store)
            .empty_message(empty)
            .empty_alignment(Alignment::Center)
            .focus(focused && self.focus == ExplorerFocus::List)
            .style(rooms_style)
            .render(list_area, buf, &mut self.list);
    }

    fn dup(&self, store: &mut ProgramStore) -> Self {
        ExplorerState {
            list: self.list.dup(store),
            filter: self.filter.dup(store),
            focus: self.focus,
            last_filter: self.last_filter.clone(),
        }
    }

    fn close(&mut self, flags: CloseFlags, store: &mut ProgramStore) -> bool {
        self.list.close(flags, store)
    }

    fn write(
        &mut self,
        path: Option<&str>,
        flags: WriteFlags,
        store: &mut ProgramStore,
    ) -> IambResult<EditInfo> {
        self.list.write(path, flags, store)
    }

    fn get_completions(&self) -> Option<CompletionList> {
        delegate_ref!(self, w => w.get_completions())
    }

    fn get_cursor_word(&self, style: &WordStyle) -> Option<String> {
        delegate_ref!(self, w => w.get_cursor_word(style))
    }

    fn get_selected_word(&self) -> Option<String> {
        delegate_ref!(self, w => w.get_selected_word())
    }
}

/// Shrink an explorer that starts off a tab's layout down to `explorer_width`.
///
/// Layouts from the configuration file split windows evenly, so this lets a tab configured as
/// `split = [{ split = [{ window = "iamb://explorer" }, { window = "iamb://chats" }] }]` open
/// with a narrow explorer on the left.
pub fn resize_docked_explorer(
    tab: &mut WindowLayoutState<IambWindow, IambInfo>,
    store: &mut ProgramStore,
) -> IambResult<()> {
    let ctx = EditContext::default();
    let first = WindowAction::Focus(FocusChange::Position(MovePosition::Beginning));
    tab.window_command(&first, &ctx, store)?;

    if tab.windows() < 2 || tab.get().is_none_or(|w| w.id() != IambId::Explorer) {
        return Ok(());
    }

    let width = store.application.settings.tunables.explorer_width;
    let resize = WindowAction::Resize(
        FocusChange::Current,
        Axis::Vertical,
        SizeChange::Exact(Count::Exact(width.into())),
    );
    tab.window_command(&resize, &ctx, store)?;

    Ok(())
}

fn is_explorer_focused(screen: &IambScreen) -> bool {
    screen.current_window().is_some_and(|w| w.id() == IambId::Explorer)
}

/// Run an [ExplorerAction] against the current tab.
pub fn explorer_command(
    act: ExplorerAction,
    screen: &mut IambScreen,
    ctx: &ProgramContext,
    store: &mut ProgramStore,
) -> IambResult<EditInfo> {
    match act {
        ExplorerAction::Toggle => explorer_toggle(screen, ctx, store),
        ExplorerAction::Open(room_id) => {
            let target = IambId::Room(room_id.into(), RoomView::Main);

            explorer_open(target, screen, ctx, store)
        },
    }
}

/// Open the explorer docked on the left side of the tab, focus it if it's already open, or close
/// it if it's already focused.
fn explorer_toggle(
    screen: &mut IambScreen,
    ctx: &ProgramContext,
    store: &mut ProgramStore,
) -> IambResult<EditInfo> {
    if is_explorer_focused(screen) {
        let close =
            WindowAction::Close(WindowTarget::Single(FocusChange::Current), CloseFlags::NONE);

        return screen.window_command(&close, ctx, store);
    }

    // The explorer always gets docked as the first window, so check there for an existing one.
    let first = WindowAction::Focus(FocusChange::Position(MovePosition::Beginning));
    screen.window_command(&first, ctx, store)?;

    if is_explorer_focused(screen) {
        return Ok(None);
    }

    let back = WindowAction::Focus(FocusChange::PreviouslyFocused);
    screen.window_command(&back, ctx, store)?;

    let width = store.application.settings.tunables.explorer_width;
    let nwins = screen.current_tab()?.windows();
    let open = WindowAction::Open(
        OpenTarget::Application(IambId::Explorer),
        OpenFlags::NONE,
        Axis::Vertical,
        MoveDir1D::Previous,
        Count::Exact(width.into()),
    );
    screen.window_command(&open, ctx, store)?;

    if nwins > 1 {
        // The explorer opened next to the focused window only, so move it out to span the
        // whole left side of the tab, and then restore its width.
        let dock = WindowAction::MoveSide(MoveDir2D::Left);
        let resize = WindowAction::Resize(
            FocusChange::Current,
            Axis::Vertical,
            SizeChange::Exact(Count::Exact(width.into())),
        );

        screen.window_command(&dock, ctx, store)?;
        screen.window_command(&resize, ctx, store)?;
    }

    Ok(None)
}

/// Show a window to the right of the focused explorer, splitting the tab when there's nothing
/// there yet. The explorer itself is never replaced.
fn explorer_open(
    target: IambId,
    screen: &mut IambScreen,
    ctx: &ProgramContext,
    store: &mut ProgramStore,
) -> IambResult<EditInfo> {
    let right = WindowAction::Focus(FocusChange::Direction2D(MoveDir2D::Right, Count::Exact(1)));
    screen.window_command(&right, ctx, store)?;

    if is_explorer_focused(screen) {
        // There's no window to the right of the explorer, so make one, and then shrink the
        // explorer back down to its usual width.
        let width = store.application.settings.tunables.explorer_width;
        let split = WindowAction::Split(
            OpenTarget::Application(target),
            OpenFlags::NONE,
            Axis::Vertical,
            MoveDir1D::Next,
            Count::Exact(1),
        );
        let resize = WindowAction::Resize(
            FocusChange::PreviouslyFocused,
            Axis::Vertical,
            SizeChange::Exact(Count::Exact(width.into())),
        );

        screen.window_command(&split, ctx, store)?;
        return screen.window_command(&resize, ctx, store);
    }

    if screen.current_window().is_some_and(|w| w.id() == target) {
        // The room is already showing, so leave its scrollback and message bar alone.
        return Ok(None);
    }

    let switch = WindowAction::Switch(OpenTarget::Application(target), OpenFlags::NONE);
    screen.window_command(&switch, ctx, store)
}

#[cfg(test)]
mod tests {
    use modalkit::ui::FocusList;
    use modalkit_ratatui::cmdbar::CommandBarState;
    use modalkit_ratatui::screen::Screen;
    use modalkit_ratatui::windows::WindowLayoutDescription;
    use serde_json::Value;

    use super::*;
    use crate::tests::mock_store;

    fn screen(store: &mut ProgramStore) -> IambScreen {
        let win = IambWindow::open(IambId::ChatList, OpenFlags::NONE, store).unwrap();
        let cmd = CommandBarState::new(store);

        ScreenState::new(win, cmd)
    }

    fn current_id(screen: &IambScreen) -> Option<IambId> {
        screen.current_window().map(|w| w.id())
    }

    fn windows(screen: &IambScreen) -> usize {
        screen.current_tab().unwrap().windows()
    }

    fn draw(screen: &mut IambScreen, store: &mut ProgramStore) {
        let area = Rect::new(0, 0, 100, 40);
        let mut buffer = Buffer::empty(area);

        Screen::new(store).render(area, &mut buffer, screen);
    }

    /// Find the width of the explorer from the last draw.
    fn explorer_width(screen: &IambScreen) -> Option<u64> {
        fn find(v: &Value) -> Option<u64> {
            if v["window"] == "iamb://explorer" {
                return v["length"].as_u64();
            }

            v["children"].as_array()?.iter().find_map(find)
        }

        let desc = screen.current_tab().unwrap().as_description();
        let desc = serde_json::to_value(desc).unwrap();

        find(&desc["layout"])
    }

    #[test]
    fn test_filter_matches() {
        assert!(filter_matches("Alice", ""));
        assert!(filter_matches("Alice", "   "));
        assert!(filter_matches("Alice", "ali"));
        assert!(filter_matches("Alice", "ALI"));
        assert!(filter_matches("Chat for devs", "dev chat"));
        assert!(filter_matches("Chat for devs", "dev\nchat"));
        assert!(!filter_matches("Chat for devs", "dev bob"));
        assert!(!filter_matches("Alice", "bob"));
    }

    #[tokio::test]
    async fn test_explorer_toggle() {
        let mut store = mock_store().await;
        let mut screen = screen(&mut store);
        let ctx = EditContext::default();
        draw(&mut screen, &mut store);

        // Toggling opens the explorer and focuses it.
        explorer_command(ExplorerAction::Toggle, &mut screen, &ctx, &mut store).unwrap();
        assert_eq!(windows(&screen), 2);
        assert_eq!(current_id(&screen), Some(IambId::Explorer));

        draw(&mut screen, &mut store);
        assert_eq!(explorer_width(&screen), Some(30));

        // The explorer is docked as the first window.
        let first = WindowAction::Focus(FocusChange::Position(MovePosition::Beginning));
        screen.window_command(&first, &ctx, &mut store).unwrap();
        assert_eq!(current_id(&screen), Some(IambId::Explorer));

        // Toggling from another window focuses the existing explorer instead of opening another.
        let next = WindowAction::Focus(FocusChange::Position(MovePosition::End));
        screen.window_command(&next, &ctx, &mut store).unwrap();
        assert_eq!(current_id(&screen), Some(IambId::ChatList));

        explorer_command(ExplorerAction::Toggle, &mut screen, &ctx, &mut store).unwrap();
        assert_eq!(windows(&screen), 2);
        assert_eq!(current_id(&screen), Some(IambId::Explorer));

        // Toggling while the explorer is focused closes it.
        explorer_command(ExplorerAction::Toggle, &mut screen, &ctx, &mut store).unwrap();
        assert_eq!(windows(&screen), 1);
        assert_eq!(current_id(&screen), Some(IambId::ChatList));
    }

    #[tokio::test]
    async fn test_explorer_toggle_splits() {
        let mut store = mock_store().await;
        let mut screen = screen(&mut store);
        let ctx = EditContext::default();

        // Start with two windows stacked on top of each other, focusing the bottom one.
        let split = WindowAction::Split(
            OpenTarget::Application(IambId::RoomList),
            OpenFlags::NONE,
            Axis::Horizontal,
            MoveDir1D::Next,
            Count::Exact(1),
        );
        screen.window_command(&split, &ctx, &mut store).unwrap();
        assert_eq!(current_id(&screen), Some(IambId::RoomList));
        draw(&mut screen, &mut store);

        // The explorer gets docked as the first window, at its usual width.
        explorer_command(ExplorerAction::Toggle, &mut screen, &ctx, &mut store).unwrap();
        assert_eq!(windows(&screen), 3);
        assert_eq!(current_id(&screen), Some(IambId::Explorer));
        draw(&mut screen, &mut store);
        assert_eq!(explorer_width(&screen), Some(30));

        let first = WindowAction::Focus(FocusChange::Position(MovePosition::Beginning));
        screen.window_command(&first, &ctx, &mut store).unwrap();
        assert_eq!(current_id(&screen), Some(IambId::Explorer));

        // Toggling again closes it and leaves the other two windows alone.
        explorer_command(ExplorerAction::Toggle, &mut screen, &ctx, &mut store).unwrap();
        assert_eq!(windows(&screen), 2);
        assert_ne!(current_id(&screen), Some(IambId::Explorer));
    }

    #[tokio::test]
    async fn test_resize_docked_explorer() {
        let mut store = mock_store().await;
        let area = Rect::new(0, 0, 100, 40);
        // Splits at the top level stack windows, so put the explorer and the chat list side by
        // side in a nested split.
        let row = WindowLayoutDescription::Split {
            children: vec![
                WindowLayoutDescription::Window { window: IambId::Explorer, length: None },
                WindowLayoutDescription::Window { window: IambId::ChatList, length: None },
            ],
            length: None,
        };
        let desc = WindowLayoutDescription::Split { children: vec![row], length: None };
        let mut tab = desc.to_layout::<IambWindow>(area.into(), &mut store).unwrap();

        resize_docked_explorer(&mut tab, &mut store).unwrap();

        let mut tabs = FocusList::default();
        tabs.push(tab);
        let mut screen = ScreenState::from_list(tabs, CommandBarState::new(&mut store));
        draw(&mut screen, &mut store);
        assert_eq!(explorer_width(&screen), Some(30));
    }

    #[tokio::test]
    async fn test_explorer_open() {
        let mut store = mock_store().await;
        let mut screen = screen(&mut store);
        let ctx = EditContext::default();
        draw(&mut screen, &mut store);

        explorer_command(ExplorerAction::Toggle, &mut screen, &ctx, &mut store).unwrap();
        assert_eq!(current_id(&screen), Some(IambId::Explorer));
        draw(&mut screen, &mut store);

        // Opening something from the explorer replaces the window to its right.
        explorer_open(IambId::Welcome, &mut screen, &ctx, &mut store).unwrap();
        assert_eq!(windows(&screen), 2);
        assert_eq!(current_id(&screen), Some(IambId::Welcome));

        // The explorer is still there.
        let back = WindowAction::Focus(FocusChange::PreviouslyFocused);
        screen.window_command(&back, &ctx, &mut store).unwrap();
        assert_eq!(current_id(&screen), Some(IambId::Explorer));

        // When the explorer is the only window, opening something splits the tab.
        let only =
            WindowAction::Close(WindowTarget::AllBut(FocusChange::Current), CloseFlags::NONE);
        screen.window_command(&only, &ctx, &mut store).unwrap();
        assert_eq!(windows(&screen), 1);
        assert_eq!(current_id(&screen), Some(IambId::Explorer));

        draw(&mut screen, &mut store);
        explorer_open(IambId::RoomList, &mut screen, &ctx, &mut store).unwrap();
        assert_eq!(windows(&screen), 2);
        assert_eq!(current_id(&screen), Some(IambId::RoomList));
        draw(&mut screen, &mut store);
        assert_eq!(explorer_width(&screen), Some(30));

        screen.window_command(&back, &ctx, &mut store).unwrap();
        assert_eq!(current_id(&screen), Some(IambId::Explorer));
    }
}
