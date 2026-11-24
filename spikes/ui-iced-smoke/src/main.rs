use iced::widget::pane_grid::{self, PaneGrid};
use iced::widget::{button, column, container, row, text};
use iced::{Element, Fill};

fn main() -> iced::Result {
    iced::application(UiSpike::default, UiSpike::update, UiSpike::view).run()
}

struct UiSpike {
    panes: pane_grid::State<PaneState>,
    next_pane_id: u64,
    next_session_id: u64,
    focused: Option<pane_grid::Pane>,
}

#[derive(Debug, Clone, Copy)]
struct PaneState {
    pane_id: u64,
    session_id: u64,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    Split(pane_grid::Axis),
    Clicked(pane_grid::Pane),
    Dragged(pane_grid::DragEvent),
    Resized(pane_grid::ResizeEvent),
    Close(pane_grid::Pane),
}

impl Default for UiSpike {
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
        }
    }
}

impl UiSpike {
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
            Message::Close(pane) => {
                if self.panes.len() > 1 {
                    if let Some((_closed, sibling)) = self.panes.close(pane) {
                        self.focused = Some(sibling);
                    }
                }
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let toolbar = row![
            button("Split horizontal").on_press(Message::Split(pane_grid::Axis::Horizontal)),
            button("Split vertical").on_press(Message::Split(pane_grid::Axis::Vertical)),
            text(format!("Panes: {}", self.panes.len())),
        ]
        .spacing(12);

        let focused = self.focused;
        let panes = PaneGrid::new(&self.panes, |id, pane, _is_maximized| {
            let label = column![
                text(format!("Pane ID: {}", pane.pane_id)).size(20),
                text(format!("Session ID: {}", pane.session_id)),
                text(if focused == Some(id) {
                    "FOCUSED"
                } else {
                    "Click to focus"
                }),
                button("Close pane").on_press(Message::Close(id)),
            ]
            .spacing(8);

            pane_grid::Content::new(container(label).padding(16).center(Fill))
        })
        .width(Fill)
        .height(Fill)
        .spacing(6)
        .on_click(Message::Clicked)
        .on_drag(Message::Dragged)
        .on_resize(10, Message::Resized);

        column![toolbar, panes].spacing(12).padding(12).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting_creates_a_new_pane_without_changing_existing_session_identity() {
        let mut app = UiSpike::default();
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
        let mut app = UiSpike::default();
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
}
