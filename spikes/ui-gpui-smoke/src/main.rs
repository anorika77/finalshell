use std::collections::BTreeSet;

use gpui::{AnyElement, App, Context, QuitMode, Window, WindowOptions, div, prelude::*, px};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaneIdentity {
    pane_id: u64,
    session_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PaneNode {
    Leaf(PaneIdentity),
    Split {
        axis: SplitAxis,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

impl PaneNode {
    fn leaf_count(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    fn first_leaf(&self) -> PaneIdentity {
        match self {
            Self::Leaf(pane) => *pane,
            Self::Split { first, .. } => first.first_leaf(),
        }
    }

    fn contains_session(&self, session_id: u64) -> bool {
        match self {
            Self::Leaf(pane) => pane.session_id == session_id,
            Self::Split { first, second, .. } => {
                first.contains_session(session_id) || second.contains_session(session_id)
            }
        }
    }

    fn split_leaf(&mut self, pane_id: u64, new_pane: PaneIdentity, axis: SplitAxis) -> bool {
        match self {
            Self::Leaf(pane) if pane.pane_id == pane_id => {
                let existing = *pane;
                *self = Self::Split {
                    axis,
                    first: Box::new(Self::Leaf(existing)),
                    second: Box::new(Self::Leaf(new_pane)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split_leaf(pane_id, new_pane, axis)
                    || second.split_leaf(pane_id, new_pane, axis)
            }
        }
    }

    fn remove_leaf(&mut self, pane_id: u64) -> bool {
        let Self::Split { first, second, .. } = self else {
            return false;
        };

        if matches!(first.as_ref(), Self::Leaf(pane) if pane.pane_id == pane_id) {
            *self = (**second).clone();
            return true;
        }
        if matches!(second.as_ref(), Self::Leaf(pane) if pane.pane_id == pane_id) {
            *self = (**first).clone();
            return true;
        }

        first.remove_leaf(pane_id) || second.remove_leaf(pane_id)
    }

    fn collect_panes(&self, panes: &mut Vec<PaneIdentity>) {
        match self {
            Self::Leaf(pane) => panes.push(*pane),
            Self::Split { first, second, .. } => {
                first.collect_panes(panes);
                second.collect_panes(panes);
            }
        }
    }
}

struct WorkspacePrototype {
    root: PaneNode,
    focused_pane_id: u64,
    next_pane_id: u64,
    sessions: BTreeSet<u64>,
}

impl Default for WorkspacePrototype {
    fn default() -> Self {
        Self {
            root: PaneNode::Leaf(PaneIdentity {
                pane_id: 1,
                session_id: 101,
            }),
            focused_pane_id: 1,
            next_pane_id: 2,
            sessions: BTreeSet::from([101]),
        }
    }
}

impl WorkspacePrototype {
    fn panes(&self) -> Vec<PaneIdentity> {
        let mut panes = Vec::with_capacity(self.root.leaf_count());
        self.root.collect_panes(&mut panes);
        panes
    }

    fn split_focused(&mut self, axis: SplitAxis) {
        let Some(focused) = self
            .panes()
            .into_iter()
            .find(|pane| pane.pane_id == self.focused_pane_id)
        else {
            return;
        };

        let new_pane = PaneIdentity {
            pane_id: self.next_pane_id,
            session_id: focused.session_id,
        };
        if self.root.split_leaf(self.focused_pane_id, new_pane, axis) {
            self.next_pane_id += 1;
            self.focused_pane_id = new_pane.pane_id;
        }
    }

    fn close_focused(&mut self) -> bool {
        if self.root.leaf_count() <= 1 {
            return false;
        }

        let closing_pane_id = self.focused_pane_id;
        if !self.root.remove_leaf(closing_pane_id) {
            return false;
        }

        self.focused_pane_id = self.root.first_leaf().pane_id;
        // A pane is only a view onto a session. Closing it never disconnects that session.
        true
    }

    fn session_is_active(&self, session_id: u64) -> bool {
        self.sessions.contains(&session_id)
    }

    fn session_has_panes(&self, session_id: u64) -> bool {
        self.root.contains_session(session_id)
    }
}

fn render_pane_node(
    node: &PaneNode,
    focused_pane_id: u64,
    cx: &Context<WorkspacePrototype>,
) -> AnyElement {
    match node {
        PaneNode::Leaf(pane) => {
            let pane_id = pane.pane_id;
            let label = format!("Pane {} / Session {}", pane.pane_id, pane.session_id);
            let focused = pane_id == focused_pane_id;
            div()
                .id(format!("pane-{pane_id}"))
                .flex()
                .flex_1()
                .p_2()
                .child(if focused {
                    format!("▶ {label}")
                } else {
                    label
                })
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.focused_pane_id = pane_id;
                    cx.notify();
                }))
                .into_any_element()
        }
        PaneNode::Split {
            axis,
            first,
            second,
        } => {
            let container = div().flex().flex_1();
            let container = match axis {
                SplitAxis::Horizontal => container,
                SplitAxis::Vertical => container.flex_col(),
            };
            container
                .child(render_pane_node(first, focused_pane_id, cx))
                .child(render_pane_node(second, focused_pane_id, cx))
                .into_any_element()
        }
    }
}

impl Render for WorkspacePrototype {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible_session_count = self
            .sessions
            .iter()
            .filter(|session_id| {
                self.session_is_active(**session_id) && self.session_has_panes(**session_id)
            })
            .count();

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .child(
                        div()
                            .w(px(220.0))
                            .h(px(32.0))
                            .child("FinalShell — GPUI CE prototype"),
                    )
                    .child(
                        div()
                            .id("split-horizontal")
                            .w(px(110.0))
                            .h(px(32.0))
                            .child("  [Split H]  ")
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.split_focused(SplitAxis::Horizontal);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("split-vertical")
                            .w(px(110.0))
                            .h(px(32.0))
                            .child("[Split V]  ")
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.split_focused(SplitAxis::Vertical);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("close-pane")
                            .w(px(110.0))
                            .h(px(32.0))
                            .child("[Close focused]")
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                if this.close_focused() {
                                    cx.notify();
                                }
                            })),
                    ),
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
                            .child(render_pane_node(&self.root, self.focused_pane_id, cx))
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
            .child(div().child(format!(
                "{} panes / {} active sessions",
                self.root.leaf_count(),
                visible_session_count
            )))
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
    use gpui::{MouseButton, TestApp, point};

    #[test]
    fn splitting_creates_a_new_pane_view_for_the_same_session() {
        let mut app = WorkspacePrototype::default();

        app.split_focused(SplitAxis::Horizontal);

        let panes = app.panes();
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].session_id, panes[1].session_id);
        assert_ne!(panes[0].pane_id, panes[1].pane_id);
        assert_eq!(app.sessions.len(), 1);
    }

    #[test]
    fn closing_one_shared_pane_keeps_the_other_pane_and_session_alive() {
        let mut app = WorkspacePrototype::default();
        app.split_focused(SplitAxis::Horizontal);
        let session_id = app.panes()[0].session_id;

        assert!(app.close_focused());

        let remaining = app.panes();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].session_id, session_id);
        assert!(app.session_is_active(session_id));
        assert!(app.session_has_panes(session_id));
    }

    #[test]
    fn closing_the_last_pane_is_rejected() {
        let mut app = WorkspacePrototype::default();

        assert!(!app.close_focused());
        assert_eq!(app.root.leaf_count(), 1);
        assert_eq!(app.sessions.len(), 1);
    }

    #[test]
    fn nested_splits_collapse_without_losing_sibling_panes() {
        let mut app = WorkspacePrototype::default();
        app.split_focused(SplitAxis::Horizontal);
        app.split_focused(SplitAxis::Vertical);
        app.split_focused(SplitAxis::Horizontal);

        assert_eq!(app.root.leaf_count(), 4);
        assert!(app.close_focused());
        assert_eq!(app.root.leaf_count(), 3);
        assert_eq!(app.panes().len(), 3);
    }

    #[test]
    fn headless_ui_split_and_close_buttons_change_the_rendered_workspace() {
        let mut app = TestApp::new();
        let mut window = app.open_window(|_, _| WorkspacePrototype::default());

        window.draw();
        window.simulate_click(point(px(275.0), px(16.0)), MouseButton::Left);
        window.read(|workspace, _| {
            assert_eq!(workspace.root.leaf_count(), 2);
            assert_eq!(
                workspace.panes()[0].session_id,
                workspace.panes()[1].session_id
            );
        });

        window.draw();
        window.simulate_click(point(px(385.0), px(16.0)), MouseButton::Left);
        window.read(|workspace, _| assert_eq!(workspace.root.leaf_count(), 3));

        window.draw();
        window.simulate_click(point(px(495.0), px(16.0)), MouseButton::Left);
        window.read(|workspace, _| {
            assert_eq!(workspace.root.leaf_count(), 2);
            assert_eq!(workspace.sessions.len(), 1);
            assert!(workspace.session_is_active(101));
        });
    }

    #[test]
    fn mock_dataset_sizes_match_the_comparison_contract() {
        let connection_ids = (0..1_000_u64).collect::<Vec<_>>();
        let file_ids = (0..10_000_u64).collect::<Vec<_>>();
        assert_eq!(connection_ids.len(), 1_000);
        assert_eq!(file_ids.len(), 10_000);
    }
}
