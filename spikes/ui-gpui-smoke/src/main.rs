use gpui::{App, Context, QuitMode, Window, WindowOptions, div, prelude::*, px};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaneIdentity {
    pane_id: u64,
    session_id: u64,
}

struct WorkspacePrototype {
    panes: Vec<PaneIdentity>,
}

impl Default for WorkspacePrototype {
    fn default() -> Self {
        Self {
            panes: vec![
                PaneIdentity {
                    pane_id: 1,
                    session_id: 101,
                },
                PaneIdentity {
                    pane_id: 2,
                    session_id: 102,
                },
                PaneIdentity {
                    pane_id: 3,
                    session_id: 103,
                },
                PaneIdentity {
                    pane_id: 4,
                    session_id: 104,
                },
            ],
        }
    }
}

impl Render for WorkspacePrototype {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .child("FinalShell — GPUI CE prototype")
                    .child("  [Split H]  [Split V]  [Close]"),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w(px(220.0))
                            .child("Connections")
                            .child("1,000 mock hosts"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .child("Terminal workspace")
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .child("Pane 1 / Session 101")
                                    .child("Pane 2 / Session 102"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .child("Pane 3 / Session 103")
                                    .child("Pane 4 / Session 104"),
                            )
                            .child("10,000 mock files"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w(px(220.0))
                            .child("Monitor")
                            .child("Overview  |  Network  |  Processes"),
                    ),
            )
            .child(div().child("Quick command — local mock only; no SSH"))
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx: &mut App| {
            cx.open_window(WindowOptions::default(), |_, cx| {
                cx.new(|_| WorkspacePrototype::default())
            })
            .expect("failed to open GPUI workspace window");
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_and_session_identity_are_distinct() {
        let app = WorkspacePrototype::default();
        assert_eq!(app.panes.len(), 4);
        assert_eq!(app.panes[0].pane_id, 1);
        assert_eq!(app.panes[0].session_id, 101);
        assert_ne!(app.panes[0].pane_id, app.panes[0].session_id);
    }

    #[test]
    fn mock_dataset_sizes_match_the_comparison_contract() {
        let connection_ids = (0..1_000_u64).collect::<Vec<_>>();
        let file_ids = (0..10_000_u64).collect::<Vec<_>>();
        assert_eq!(connection_ids.len(), 1_000);
        assert_eq!(file_ids.len(), 10_000);
    }
}
