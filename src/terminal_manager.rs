use std::collections::BTreeMap;

use crate::{
    ssh::{SshClient, SshError},
    terminal::{TerminalSession},
    terminal_screen::{TerminalCharset, TerminalScreen},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaneNode {
    Terminal(u64),
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaneLayout {
    root: PaneNode,
    focused: u64,
}

impl PaneLayout {
    pub fn new(terminal_id: u64) -> Self {
        Self {
            root: PaneNode::Terminal(terminal_id),
            focused: terminal_id,
        }
    }

    pub fn root(&self) -> &PaneNode {
        &self.root
    }

    pub fn focused(&self) -> u64 {
        self.focused
    }

    pub fn focus(&mut self, terminal_id: u64) -> bool {
        if contains(&self.root, terminal_id) {
            self.focused = terminal_id;
            true
        } else {
            false
        }
    }

    pub fn split_focused(&mut self, new_terminal_id: u64, axis: SplitAxis, ratio: f32) -> bool {
        if contains(&self.root, new_terminal_id) {
            return false;
        }
        let old_focused = self.focused;
        let replacement = PaneNode::Split {
            axis,
            ratio: ratio.clamp(0.1, 0.9),
            first: Box::new(PaneNode::Terminal(old_focused)),
            second: Box::new(PaneNode::Terminal(new_terminal_id)),
        };
        if !replace_leaf(&mut self.root, old_focused, replacement) {
            return false;
        }
        self.focused = new_terminal_id;
        true
    }

    pub fn close_focused(&mut self) -> Option<u64> {
        let removed = self.focused;
        let (root, found) = remove_leaf(self.root.clone(), removed);
        if !found {
            return None;
        }
        self.root = root?;
        self.focused = first_terminal(&self.root)?;
        Some(removed)
    }

    pub fn swap(&mut self, first_id: u64, second_id: u64) -> bool {
        if first_id == second_id
            || !contains(&self.root, first_id)
            || !contains(&self.root, second_id)
        {
            return false;
        }
        swap_leaves(&mut self.root, first_id, second_id);
        true
    }

    pub fn terminal_ids(&self) -> Vec<u64> {
        let mut ids = Vec::new();
        collect_ids(&self.root, &mut ids);
        ids
    }
}

fn contains(node: &PaneNode, id: u64) -> bool {
    match node {
        PaneNode::Terminal(current) => *current == id,
        PaneNode::Split { first, second, .. } => contains(first, id) || contains(second, id),
    }
}

fn replace_leaf(node: &mut PaneNode, id: u64, replacement: PaneNode) -> bool {
    match node {
        PaneNode::Terminal(current) if *current == id => {
            *node = replacement;
            true
        }
        PaneNode::Split { first, second, .. } => {
            replace_leaf(first, id, replacement.clone()) || replace_leaf(second, id, replacement)
        }
        _ => false,
    }
}

fn remove_leaf(node: PaneNode, id: u64) -> (Option<PaneNode>, bool) {
    match node {
        PaneNode::Terminal(current) if current == id => (None, true),
        PaneNode::Terminal(_) => (Some(node), false),
        PaneNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let (new_first, removed_first) = remove_leaf(*first, id);
            if removed_first {
                return (
                    Some(match new_first {
                        Some(first) => PaneNode::Split {
                            axis,
                            ratio,
                            first: Box::new(first),
                            second,
                        },
                        None => *second,
                    }),
                    true,
                );
            }
            let (new_second, removed_second) = remove_leaf(*second, id);
            if removed_second {
                return (
                    Some(match new_second {
                        Some(second) => PaneNode::Split {
                            axis,
                            ratio,
                            first: Box::new(new_first.expect("first pane remains")),
                            second: Box::new(second),
                        },
                        None => new_first.expect("first pane remains"),
                    }),
                    true,
                );
            }
            (
                Some(PaneNode::Split {
                    axis,
                    ratio,
                    first: Box::new(new_first.expect("first pane remains")),
                    second: Box::new(new_second.expect("second pane remains")),
                }),
                false,
            )
        }
    }
}

fn first_terminal(node: &PaneNode) -> Option<u64> {
    match node {
        PaneNode::Terminal(id) => Some(*id),
        PaneNode::Split { first, .. } => first_terminal(first),
    }
}

fn collect_ids(node: &PaneNode, ids: &mut Vec<u64>) {
    match node {
        PaneNode::Terminal(id) => ids.push(*id),
        PaneNode::Split { first, second, .. } => {
            collect_ids(first, ids);
            collect_ids(second, ids);
        }
    }
}

fn swap_leaves(node: &mut PaneNode, first_id: u64, second_id: u64) {
    match node {
        PaneNode::Terminal(id) if *id == first_id => *id = second_id,
        PaneNode::Terminal(id) if *id == second_id => *id = first_id,
        PaneNode::Split { first, second, .. } => {
            swap_leaves(first, first_id, second_id);
            swap_leaves(second, first_id, second_id);
        }
        _ => {}
    }
}

/// Owns multiple independent PTY channels over one authenticated SSH connection.
pub struct TerminalManager {
    sessions: BTreeMap<u64, TerminalSession>,
    screens: BTreeMap<u64, TerminalScreen>,
    active: Option<u64>,
    layout: Option<PaneLayout>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            screens: BTreeMap::new(),
            active: None,
            layout: None,
        }
    }

    pub async fn open(
        &mut self,
        ssh: &SshClient,
        term: &str,
        cols: u32,
        rows: u32,
    ) -> Result<u64, SshError> {
        let session = ssh.open_terminal(term, cols, rows).await?;
        let id = session.id();
        self.sessions.insert(id, session);
        self.screens.insert(
            id,
            TerminalScreen::new(rows.min(u16::MAX as u32) as u16, cols.min(u16::MAX as u32) as u16, 10_000),
        );
        self.active = Some(id);
        if self.layout.is_none() {
            self.layout = Some(PaneLayout::new(id));
        }
        Ok(id)
    }

    pub async fn split(
        &mut self,
        ssh: &SshClient,
        axis: SplitAxis,
        ratio: f32,
        term: &str,
        cols: u32,
        rows: u32,
    ) -> Result<u64, SshError> {
        let new_id = self.open(ssh, term, cols, rows).await?;
        let layout = self
            .layout
            .as_mut()
            .expect("opening a terminal creates a layout");
        if !layout.split_focused(new_id, axis, ratio) {
            self.sessions.remove(&new_id);
            self.screens.remove(&new_id);
            self.active = layout.focused().into();
            return Err(SshError::Proxy(
                "cannot split the active terminal pane".into(),
            ));
        }
        Ok(new_id)
    }

    pub fn focus(&mut self, terminal_id: u64) -> bool {
        if !self.sessions.contains_key(&terminal_id) {
            return false;
        }
        if let Some(layout) = &mut self.layout {
            if !layout.focus(terminal_id) {
                return false;
            }
        }
        self.active = Some(terminal_id);
        true
    }

    pub fn active_id(&self) -> Option<u64> {
        self.active
    }

    pub fn ids(&self) -> Vec<u64> {
        self.sessions.keys().copied().collect()
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn layout(&self) -> Option<&PaneLayout> {
        self.layout.as_ref()
    }

    pub fn screen(&self, terminal_id: u64) -> Option<&TerminalScreen> {
        self.screens.get(&terminal_id)
    }

    pub fn screen_mut(&mut self, terminal_id: u64) -> Option<&mut TerminalScreen> {
        self.screens.get_mut(&terminal_id)
    }

    pub fn process_output(&mut self, terminal_id: u64, bytes: &[u8]) -> bool {
        if let Some(screen) = self.screens.get_mut(&terminal_id) {
            screen.process(bytes);
            true
        } else {
            false
        }
    }

    pub async fn resize(
        &mut self,
        terminal_id: u64,
        cols: u32,
        rows: u32,
    ) -> Result<bool, SshError> {
        let Some(session) = self.sessions.get_mut(&terminal_id) else {
            return Ok(false);
        };
        session.resize(cols, rows).await?;
        if let Some(screen) = self.screens.get_mut(&terminal_id) {
            screen.resize(rows.min(u16::MAX as u32) as u16, cols.min(u16::MAX as u32) as u16);
        }
        Ok(true)
    }

    pub fn set_charset(&mut self, terminal_id: u64, charset: TerminalCharset) -> bool {
        if let Some(screen) = self.screens.get_mut(&terminal_id) {
            screen.set_charset(charset);
            true
        } else {
            false
        }
    }

    pub fn get(&self, terminal_id: u64) -> Option<&TerminalSession> {
        self.sessions.get(&terminal_id)
    }

    pub fn get_mut(&mut self, terminal_id: u64) -> Option<&mut TerminalSession> {
        self.sessions.get_mut(&terminal_id)
    }

    pub fn active_mut(&mut self) -> Option<&mut TerminalSession> {
        self.active.and_then(|id| self.sessions.get_mut(&id))
    }

    pub async fn close(&mut self, terminal_id: u64) -> Result<bool, SshError> {
        let Some(mut session) = self.sessions.remove(&terminal_id) else {
            return Ok(false);
        };
        session.close().await?;
        self.screens.remove(&terminal_id);
        if let Some(layout) = &mut self.layout {
            if layout.focused() == terminal_id {
                let _ = layout.close_focused();
            } else {
                let previous = layout.focused();
                if layout.focus(terminal_id) {
                    let _ = layout.close_focused();
                    layout.focus(previous);
                }
            }
        }
        if self.active == Some(terminal_id) {
            self.active = self.layout.as_ref().map(PaneLayout::focused);
        }
        if self.sessions.is_empty() {
            self.active = None;
            self.layout = None;
        }
        Ok(true)
    }

    pub async fn close_all(&mut self) {
        for (_, mut session) in std::mem::take(&mut self.sessions) {
            let _ = session.close().await;
        }
        self.screens.clear();
        self.active = None;
        self.layout = None;
    }
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_layout_splits_focuses_swaps_and_collapses() {
        let mut layout = PaneLayout::new(10);
        assert!(layout.split_focused(11, SplitAxis::Vertical, 0.5));
        assert_eq!(layout.focused(), 11);
        assert!(layout.focus(10));
        assert!(layout.swap(10, 11));
        assert_eq!(layout.terminal_ids(), vec![11, 10]);
        assert_eq!(layout.close_focused(), Some(10));
        assert_eq!(layout.terminal_ids(), vec![11]);
        assert_eq!(layout.focused(), 11);
    }
}
