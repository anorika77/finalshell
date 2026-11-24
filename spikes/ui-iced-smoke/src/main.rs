use iced::widget::pane_grid::{self, PaneGrid};
use iced::widget::{Column, button, column, container, row, scrollable, space, text, text_input};
use iced::{Element, Fill, Length};

fn main() -> iced::Result {
    iced::application(
        WorkspacePrototype::default,
        WorkspacePrototype::update,
        WorkspacePrototype::view,
    )
    .title("FinalShell — UI Workspace Prototype")
    .subscription(WorkspacePrototype::subscription)
    .run()
}

#[derive(Debug, Clone)]
struct ConnectionNode {
    id: u32,
    name: String,
    group: String,
}

#[derive(Debug, Clone)]
struct FileEntry {
    name: String,
    size: String,
    kind: &'static str,
}

const FILE_ROW_HEIGHT: f32 = 24.0;
const FILE_LIST_HEIGHT: f32 = 156.0;
const FILE_LIST_OVERSCAN: usize = 3;
const CONNECTION_ROW_HEIGHT: f32 = 26.0;
const CONNECTION_LIST_OVERSCAN: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MonitorTab {
    Overview,
    Network,
    Processes,
}

struct WorkspacePrototype {
    panes: pane_grid::State<PaneState>,
    next_pane_id: u64,
    next_session_id: u64,
    focused: Option<pane_grid::Pane>,
    connections: Vec<ConnectionNode>,
    connection_filter: String,
    connection_scroll_offset: f32,
    connection_viewport_height: f32,
    selected_connection: Option<u32>,
    monitor_tab: MonitorTab,
    quick_command: String,
    last_mock_command: Option<String>,
    files: Vec<FileEntry>,
    file_scroll_offset: f32,
    file_viewport_height: f32,
    selected_file: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
struct PaneState {
    pane_id: u64,
    session_id: u64,
}

#[derive(Debug, Clone)]
enum Message {
    Split(pane_grid::Axis),
    Clicked(pane_grid::Pane),
    Dragged(pane_grid::DragEvent),
    Resized(pane_grid::ResizeEvent),
    Close(pane_grid::Pane),
    CloseFocused,
    ConnectionFilterChanged(String),
    ConnectionListScrolled { offset: f32, viewport_height: f32 },
    SelectConnection(u32),
    SelectMonitor(MonitorTab),
    QuickCommandChanged(String),
    SubmitQuickCommand,
    FileListScrolled { offset: f32, viewport_height: f32 },
    SelectFile(usize),
}

impl Default for WorkspacePrototype {
    fn default() -> Self {
        let (panes, first) = pane_grid::State::new(PaneState {
            pane_id: 1,
            session_id: 1001,
        });

        Self {
            panes,
            next_pane_id: 2,
            next_session_id: 1002,
            focused: Some(first),
            connections: demo_connections(),
            connection_filter: String::new(),
            connection_scroll_offset: 0.0,
            connection_viewport_height: 260.0,
            selected_connection: None,
            monitor_tab: MonitorTab::Overview,
            quick_command: String::new(),
            last_mock_command: None,
            files: demo_files(),
            file_scroll_offset: 0.0,
            file_viewport_height: FILE_LIST_HEIGHT,
            selected_file: None,
        }
    }
}

fn demo_files() -> Vec<FileEntry> {
    (0..10_000)
        .map(|index| {
            let extension = match index % 4 {
                0 => "log",
                1 => "conf",
                2 => "tar.gz",
                _ => "txt",
            };
            FileEntry {
                name: format!("remote-file-{index:05}.{extension}"),
                size: format!("{} KB", (index * 37 % 8192) + 1),
                kind: if index % 4 == 2 { "archive" } else { "file" },
            }
        })
        .collect()
}

fn visible_file_range(
    total: usize,
    offset: f32,
    viewport_height: f32,
    row_height: f32,
    overscan: usize,
) -> std::ops::Range<usize> {
    if total == 0 {
        return 0..0;
    }

    let row_height = row_height.max(1.0);
    let viewport_height = viewport_height.max(0.0);
    let content_height = total as f32 * row_height;
    let max_offset = (content_height - viewport_height).max(0.0);
    let clamped_offset = offset.max(0.0).min(max_offset);
    let first_visible = (clamped_offset / row_height).floor() as usize;
    let visible_count = (viewport_height.max(row_height) / row_height).ceil() as usize;
    let start = first_visible.saturating_sub(overscan);
    let end = first_visible
        .saturating_add(visible_count)
        .saturating_add(overscan)
        .min(total);

    start.min(total)..end.max(start.min(total))
}

fn shortcut_message(
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Option<Message> {
    use iced::keyboard::Key;

    if !modifiers.command() {
        return None;
    }

    match key {
        Key::Character(character) if modifiers.shift() => {
            if character.eq_ignore_ascii_case("h") {
                Some(Message::Split(pane_grid::Axis::Horizontal))
            } else if character.eq_ignore_ascii_case("v") {
                Some(Message::Split(pane_grid::Axis::Vertical))
            } else if character.eq_ignore_ascii_case("w") {
                Some(Message::CloseFocused)
            } else {
                None
            }
        }
        Key::Character(character) if modifiers.alt() => match character.as_str() {
            "1" => Some(Message::SelectMonitor(MonitorTab::Overview)),
            "2" => Some(Message::SelectMonitor(MonitorTab::Network)),
            "3" => Some(Message::SelectMonitor(MonitorTab::Processes)),
            _ => None,
        },
        _ => None,
    }
}

fn connection_matches(node: &ConnectionNode, filter: &str) -> bool {
    let needle = filter.trim().to_lowercase();
    needle.is_empty()
        || node.name.to_lowercase().contains(&needle)
        || node.group.to_lowercase().contains(&needle)
}

fn demo_connections() -> Vec<ConnectionNode> {
    (1..=1_000)
        .map(|id| ConnectionNode {
            id,
            name: format!("server-{id:04}"),
            group: format!("Group {}", (id - 1) / 100 + 1),
        })
        .collect()
}

impl WorkspacePrototype {
    fn subscription(&self) -> iced::Subscription<Message> {
        use iced::keyboard;

        keyboard::listen().filter_map(|event| {
            let keyboard::Event::KeyPressed {
                key,
                modifiers,
                repeat: false,
                ..
            } = event
            else {
                return None;
            };

            shortcut_message(&key, modifiers)
        })
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Split(axis) => {
                if let Some(source) = self.focused {
                    let new_state = PaneState {
                        pane_id: self.next_pane_id,
                        session_id: self.next_session_id,
                    };
                    if let Some((new_pane, _split)) = self.panes.split(axis, source, new_state) {
                        self.focused = Some(new_pane);
                        self.next_pane_id += 1;
                        self.next_session_id += 1;
                    }
                }
            }
            Message::Clicked(pane) => self.focused = Some(pane),
            Message::Dragged(pane_grid::DragEvent::Dropped { pane, target }) => {
                self.panes.drop(pane, target);
            }
            Message::Dragged(_) => {}
            Message::Resized(pane_grid::ResizeEvent { split, ratio }) => {
                self.panes.resize(split, ratio);
            }
            Message::Close(pane) => self.close_pane(pane),
            Message::CloseFocused => {
                if let Some(pane) = self.focused {
                    self.close_pane(pane);
                }
            }
            Message::ConnectionFilterChanged(value) => self.connection_filter = value,
            Message::ConnectionListScrolled {
                offset,
                viewport_height,
            } => {
                self.connection_scroll_offset = offset;
                self.connection_viewport_height = viewport_height;
            }
            Message::SelectConnection(id) => self.selected_connection = Some(id),
            Message::SelectMonitor(tab) => self.monitor_tab = tab,
            Message::QuickCommandChanged(value) => self.quick_command = value,
            Message::SubmitQuickCommand => {
                let command = self.quick_command.trim();
                if !command.is_empty() {
                    self.last_mock_command = Some(command.to_owned());
                    self.quick_command.clear();
                }
            }
            Message::FileListScrolled {
                offset,
                viewport_height,
            } => {
                self.file_scroll_offset = offset;
                self.file_viewport_height = viewport_height;
            }
            Message::SelectFile(index) => self.selected_file = Some(index),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let toolbar = row![
            text("FinalShell").size(20),
            button("Split ↔").on_press(Message::Split(pane_grid::Axis::Horizontal)),
            button("Split ↕").on_press(Message::Split(pane_grid::Axis::Vertical)),
            text(format!("{} panes", self.panes.len())),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);

        let sidebar = self.connection_sidebar();
        let workspace = self.terminal_workspace();
        let monitor = self.monitor_panel();

        let main_area = row![
            container(sidebar).width(240).height(Fill),
            container(workspace).width(Fill).height(Fill),
            container(monitor).width(230).height(Fill),
        ]
        .spacing(8)
        .height(Fill);

        let file_panel = self.file_panel();

        let quick_bar = row![
            text("Quick command"),
            text_input("Mock only — no SSH command is sent", &self.quick_command)
                .on_input(Message::QuickCommandChanged)
                .on_submit(Message::SubmitQuickCommand)
                .width(Fill),
            button("Submit").on_press(Message::SubmitQuickCommand),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let status = text(match &self.last_mock_command {
            Some(command) => format!("Prototype mode · last local mock command: {command}"),
            None => "Prototype mode · no SSH connection · deterministic fixture data".to_owned(),
        })
        .size(12);

        column![toolbar, main_area, file_panel, quick_bar, status]
            .spacing(8)
            .padding(10)
            .into()
    }

    fn close_pane(&mut self, pane: pane_grid::Pane) {
        if self.panes.len() > 1 {
            if let Some((_closed, sibling)) = self.panes.close(pane) {
                self.focused = Some(sibling);
            }
        }
    }

    fn connection_sidebar(&self) -> Element<'_, Message> {
        let filtered: Vec<&ConnectionNode> = self
            .connections
            .iter()
            .filter(|node| connection_matches(node, &self.connection_filter))
            .collect();
        let range = visible_file_range(
            filtered.len(),
            self.connection_scroll_offset,
            self.connection_viewport_height,
            CONNECTION_ROW_HEIGHT,
            CONNECTION_LIST_OVERSCAN,
        );

        let mut visible_rows: Column<'_, Message> = column![];
        for node in &filtered[range.clone()] {
            let label = match self.selected_connection {
                Some(id) if id == node.id => format!("● {}  {}", node.name, node.group),
                _ => format!("{}  {}", node.name, node.group),
            };
            visible_rows = visible_rows.push(
                button(text(label).size(13))
                    .width(Fill)
                    .height(Length::Fixed(CONNECTION_ROW_HEIGHT))
                    .padding(2)
                    .on_press(Message::SelectConnection(node.id)),
            );
        }

        let top_padding = range.start as f32 * CONNECTION_ROW_HEIGHT;
        let bottom_padding =
            filtered.len().saturating_sub(range.end) as f32 * CONNECTION_ROW_HEIGHT;
        let list_content = column![
            space().height(Length::Fixed(top_padding)),
            visible_rows,
            space().height(Length::Fixed(bottom_padding)),
        ];
        let list = scrollable(list_content).height(Fill).on_scroll(|viewport| {
            Message::ConnectionListScrolled {
                offset: viewport.absolute_offset().y,
                viewport_height: viewport.bounds().height,
            }
        });

        let header = column![
            text("Connections").size(17),
            text_input("Filter 1,000 demo hosts…", &self.connection_filter)
                .on_input(Message::ConnectionFilterChanged),
        ]
        .spacing(8);
        let footer = text(format!(
            "{} / {} hosts · rendering {} rows",
            filtered.len(),
            self.connections.len(),
            range.len()
        ))
        .size(11);

        container(column![header, list, footer].spacing(8))
            .padding(8)
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn terminal_workspace(&self) -> Element<'_, Message> {
        let focused = self.focused;
        PaneGrid::new(&self.panes, |id, pane, _is_maximized| {
            let focus_label = if focused == Some(id) { "ACTIVE" } else { " " };
            let fixture = terminal_fixture(pane.session_id);
            let mut lines: Column<'_, Message> = column![
                row![
                    text(format!("Pane {}", pane.pane_id)).size(14),
                    text(format!("Session {}", pane.session_id)).size(12),
                    text(focus_label).size(11),
                    button("×").on_press(Message::Close(id)),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            ]
            .spacing(6);

            for line in fixture {
                lines = lines.push(text(line).size(13));
            }

            pane_grid::Content::new(
                container(scrollable(lines).height(Fill))
                    .padding(10)
                    .width(Fill)
                    .height(Fill),
            )
        })
        .width(Fill)
        .height(Fill)
        .spacing(6)
        .min_size(120)
        .on_click(Message::Clicked)
        .on_drag(Message::Dragged)
        .on_resize(10, Message::Resized)
        .into()
    }

    fn file_panel(&self) -> Element<'_, Message> {
        let range = visible_file_range(
            self.files.len(),
            self.file_scroll_offset,
            self.file_viewport_height,
            FILE_ROW_HEIGHT,
            FILE_LIST_OVERSCAN,
        );

        let mut visible_rows: Column<'_, Message> = column![];
        for index in range.clone() {
            let file = &self.files[index];
            let selected = self.selected_file == Some(index);
            let marker = if selected { "●" } else { " " };
            visible_rows = visible_rows.push(
                button(
                    row![
                        text(format!("{marker} {}", file.name)).size(12).width(Fill),
                        text(&file.size).size(11),
                        text(file.kind).size(11),
                    ]
                    .spacing(12),
                )
                .width(Fill)
                .height(Length::Fixed(FILE_ROW_HEIGHT))
                .padding(2)
                .on_press(Message::SelectFile(index)),
            );
        }

        let top_padding = range.start as f32 * FILE_ROW_HEIGHT;
        let bottom_padding = self.files.len().saturating_sub(range.end) as f32 * FILE_ROW_HEIGHT;
        let list_content = column![
            space().height(Length::Fixed(top_padding)),
            visible_rows,
            space().height(Length::Fixed(bottom_padding)),
        ];

        let list = scrollable(list_content)
            .height(Length::Fixed(FILE_LIST_HEIGHT))
            .on_scroll(|viewport| {
                let offset = viewport.absolute_offset().y;
                let viewport_height = viewport.bounds().height;
                Message::FileListScrolled {
                    offset,
                    viewport_height,
                }
            });

        let header = row![
            text("Remote files (10,000 fixture rows)").size(14),
            text(format!(
                "Rendering {} rows · {}–{} / {}",
                range.len(),
                range.start.saturating_add(1),
                range.end,
                self.files.len()
            ))
            .size(11),
        ]
        .spacing(12);

        container(column![header, list].spacing(6))
            .padding(8)
            .width(Fill)
            .into()
    }

    fn monitor_panel(&self) -> Element<'_, Message> {
        let tabs = column![
            text("Monitor").size(17),
            button("Overview")
                .width(Fill)
                .on_press(Message::SelectMonitor(MonitorTab::Overview)),
            button("Network")
                .width(Fill)
                .on_press(Message::SelectMonitor(MonitorTab::Network)),
            button("Processes")
                .width(Fill)
                .on_press(Message::SelectMonitor(MonitorTab::Processes)),
        ]
        .spacing(6);

        let body = match self.monitor_tab {
            MonitorTab::Overview => column![
                text("CPU").size(13),
                text("12.4%"),
                text("Memory").size(13),
                text("4.2 / 16 GB"),
                text("Uptime").size(13),
                text("12 days"),
            ],
            MonitorTab::Network => column![
                text("RX").size(13),
                text("1.8 MB/s"),
                text("TX").size(13),
                text("420 KB/s"),
                text("Interface").size(13),
                text("eth0"),
            ],
            MonitorTab::Processes => column![
                text("PID   PROCESS").size(13),
                text("1024  sshd"),
                text("1180  bash"),
                text("2201  nginx"),
            ],
        }
        .spacing(8);

        container(column![tabs, body].spacing(16))
            .padding(8)
            .width(Fill)
            .height(Fill)
            .into()
    }
}

fn terminal_fixture(session_id: u64) -> Vec<String> {
    vec![
        format!("fixture-session-{session_id} $ uname -a"),
        "Linux demo-host 6.8.0 x86_64 GNU/Linux".to_owned(),
        format!("fixture-session-{session_id} $ printf '你好，FinalShell 👋\\n'"),
        "你好，FinalShell 👋".to_owned(),
        "VT fixture: 颜色 / Unicode / CJK / emoji".to_owned(),
        format!("fixture-session-{session_id} $ _"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_shortcuts_are_platform_command_key_aware() {
        use iced::keyboard::{Key, Modifiers};

        let split = shortcut_message(
            &Key::Character("h".into()),
            Modifiers::COMMAND.union(Modifiers::SHIFT),
        );
        assert!(matches!(
            split,
            Some(Message::Split(pane_grid::Axis::Horizontal))
        ));

        let close = shortcut_message(
            &Key::Character("w".into()),
            Modifiers::COMMAND.union(Modifiers::SHIFT),
        );
        assert!(matches!(close, Some(Message::CloseFocused)));

        let monitor = shortcut_message(
            &Key::Character("2".into()),
            Modifiers::COMMAND.union(Modifiers::ALT),
        );
        assert!(matches!(
            monitor,
            Some(Message::SelectMonitor(MonitorTab::Network))
        ));
    }

    #[test]
    fn shortcuts_without_platform_command_modifier_are_ignored() {
        use iced::keyboard::{Key, Modifiers};

        assert!(shortcut_message(&Key::Character("h".into()), Modifiers::SHIFT).is_none());
        assert!(shortcut_message(&Key::Character("x".into()), Modifiers::COMMAND).is_none());
    }

    #[test]
    fn splitting_creates_a_new_pane_without_changing_existing_session_identity() {
        let mut app = WorkspacePrototype::default();
        let original_pane = app.focused.expect("initial pane");
        let original_session = app.panes.get(original_pane).expect("pane state").session_id;

        app.update(Message::Split(pane_grid::Axis::Vertical));

        assert_eq!(app.panes.len(), 2);
        assert_eq!(
            app.panes
                .get(original_pane)
                .expect("original pane remains")
                .session_id,
            original_session
        );

        let new_pane = app.focused.expect("new pane focused");
        assert_ne!(new_pane, original_pane);
        assert_ne!(
            app.panes.get(new_pane).expect("new pane state").session_id,
            original_session
        );
    }

    #[test]
    fn closing_one_pane_keeps_the_other_pane_and_its_session() {
        let mut app = WorkspacePrototype::default();
        let original_pane = app.focused.expect("initial pane");
        let original_session = app.panes.get(original_pane).expect("pane state").session_id;

        app.update(Message::Split(pane_grid::Axis::Horizontal));
        let new_pane = app.focused.expect("new pane focused");
        app.update(Message::Close(new_pane));

        assert_eq!(app.panes.len(), 1);
        assert_eq!(
            app.panes
                .get(original_pane)
                .expect("original pane remains")
                .session_id,
            original_session
        );
    }

    #[test]
    fn connection_fixture_contains_one_thousand_searchable_hosts() {
        let app = WorkspacePrototype::default();
        assert_eq!(app.connections.len(), 1_000);
        assert!(
            app.connections
                .iter()
                .any(|node| node.name == "server-0999")
        );
    }

    #[test]
    fn connection_filter_returns_expected_hosts_and_groups() {
        let connections = demo_connections();
        let server_match = connections
            .iter()
            .filter(|node| connection_matches(node, "server-0999"))
            .map(|node| node.id)
            .collect::<Vec<_>>();
        let group_match = connections
            .iter()
            .filter(|node| connection_matches(node, "Group 10"))
            .count();

        assert_eq!(server_match, vec![999]);
        assert_eq!(group_match, 100);
        assert_eq!(
            connections
                .iter()
                .filter(|node| connection_matches(node, "no-such-host"))
                .count(),
            0
        );
    }

    #[test]
    fn connection_virtualization_clamps_when_filter_shrinks_the_dataset() {
        let range = visible_file_range(
            3,
            99_999.0,
            130.0,
            CONNECTION_ROW_HEIGHT,
            CONNECTION_LIST_OVERSCAN,
        );

        assert_eq!(range, 0..3);
    }

    #[test]
    fn connection_selection_is_stable_by_host_id() {
        let mut app = WorkspacePrototype::default();
        app.update(Message::SelectConnection(999));
        app.update(Message::ConnectionFilterChanged("server-0999".to_owned()));

        let selected = app
            .connections
            .iter()
            .find(|node| connection_matches(node, &app.connection_filter))
            .expect("filtered host exists");

        assert_eq!(
            selected.id,
            app.selected_connection.expect("selection retained")
        );
    }

    #[test]
    fn file_fixture_has_ten_thousand_rows() {
        let app = WorkspacePrototype::default();
        assert_eq!(app.files.len(), 10_000);
        assert_eq!(app.files[9_999].name, "remote-file-09999.txt");
    }

    #[test]
    fn file_virtualization_materializes_only_a_viewport_sized_window() {
        let range =
            visible_file_range(10_000, 24_000.0, 156.0, FILE_ROW_HEIGHT, FILE_LIST_OVERSCAN);
        assert_eq!(range.start, 997);
        assert!(range.len() <= 20);
        assert!(range.contains(&1_000));
    }

    #[test]
    fn file_virtualization_clamps_at_the_end_of_the_dataset() {
        let range = visible_file_range(
            10_000,
            999_999.0,
            156.0,
            FILE_ROW_HEIGHT,
            FILE_LIST_OVERSCAN,
        );
        assert_eq!(range.end, 10_000);
        assert!(range.start < range.end);
    }

    #[test]
    fn empty_file_virtualization_range_is_empty() {
        assert_eq!(
            visible_file_range(0, 0.0, FILE_LIST_HEIGHT, FILE_ROW_HEIGHT, 3),
            0..0
        );
    }

    #[test]
    fn quick_command_is_recorded_locally_and_never_sent_to_a_remote_session() {
        let mut app = WorkspacePrototype::default();
        app.quick_command = "echo hello".to_owned();

        app.update(Message::SubmitQuickCommand);

        assert_eq!(app.last_mock_command.as_deref(), Some("echo hello"));
        assert!(app.quick_command.is_empty());
    }
}
