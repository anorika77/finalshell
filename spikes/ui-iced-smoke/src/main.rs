use iced::widget::pane_grid::{self, PaneGrid};
use iced::widget::{
    button, column, container, row, scrollable, text, text_input, Column,
};
use iced::{Element, Fill, Length};

fn main() -> iced::Result {
    iced::application(WorkspacePrototype::default, WorkspacePrototype::update, WorkspacePrototype::view)
        .title("FinalShell — UI Workspace Prototype")
        .run()
}

#[derive(Debug, Clone)]
struct ConnectionNode {
    id: u32,
    name: String,
    group: String,
}

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
    selected_connection: Option<u32>,
    monitor_tab: MonitorTab,
    quick_command: String,
    last_mock_command: Option<String>,
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
    ConnectionFilterChanged(String),
    SelectConnection(u32),
    SelectMonitor(MonitorTab),
    QuickCommandChanged(String),
    SubmitQuickCommand,
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
            selected_connection: None,
            monitor_tab: MonitorTab::Overview,
            quick_command: String::new(),
            last_mock_command: None,
        }
    }
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
    fn update(&mut self, message: Message) {
        match message {
            Message::Split(axis) => {
                if let Some(source) = self.focused {
                    let new_state = PaneState {
                        pane_id: self.next_pane_id,
                        session_id: self.next_session_id,
                    };
                    if let Some((new_pane, _split)) =
                        self.panes.split(axis, source, new_state)
                    {
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
            Message::Close(pane) => {
                if self.panes.len() > 1 {
                    if let Some((_closed, sibling)) = self.panes.close(pane) {
                        self.focused = Some(sibling);
                    }
                }
            }
            Message::ConnectionFilterChanged(value) => self.connection_filter = value,
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

        column![toolbar, main_area, quick_bar, status]
            .spacing(8)
            .padding(10)
            .into()
    }

    fn connection_sidebar(&self) -> Element<'_, Message> {
        let mut items: Column<'_, Message> = column![
            text("Connections").size(17),
            text_input("Filter 1,000 demo hosts…", &self.connection_filter)
                .on_input(Message::ConnectionFilterChanged),
        ]
        .spacing(8);

        let needle = self.connection_filter.to_lowercase();
        let mut visible = 0usize;

        for node in &self.connections {
            if !needle.is_empty()
                && !node.name.to_lowercase().contains(&needle)
                && !node.group.to_lowercase().contains(&needle)
            {
                continue;
            }

            visible += 1;
            let label = match self.selected_connection {
                Some(id) if id == node.id => format!("● {}  {}", node.name, node.group),
                _ => format!("{}  {}", node.name, node.group),
            };
            items = items.push(
                button(text(label).size(13))
                    .width(Fill)
                    .on_press(Message::SelectConnection(node.id)),
            );
        }

        items = items.push(text(format!("{visible} / {} hosts", self.connections.len())).size(11));

        container(scrollable(items).height(Fill))
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

    fn monitor_panel(&self) -> Element<'_, Message> {
        let tabs = column![
            text("Monitor").size(17),
            button("Overview").width(Fill).on_press(Message::SelectMonitor(MonitorTab::Overview)),
            button("Network").width(Fill).on_press(Message::SelectMonitor(MonitorTab::Network)),
            button("Processes").width(Fill).on_press(Message::SelectMonitor(MonitorTab::Processes)),
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
        assert!(app.connections.iter().any(|node| node.name == "server-0999"));
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
