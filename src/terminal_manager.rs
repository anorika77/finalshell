use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    ssh::{SshClient, SshError},
    terminal::TerminalSession,
    terminal_screen::{TerminalCharset, TerminalScreen},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum PaneNode {
    Terminal(u64),
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(data: &str) -> Result<Self, String> {
        let layout: Self = serde_json::from_str(data).map_err(|e| e.to_string())?;
        layout.validate()?;
        Ok(layout)
    }

    pub fn validate(&self) -> Result<(), String> {
        let mut ids = self.terminal_ids();
        ids.sort_unstable();
        if ids.is_empty() {
            return Err("pane layout cannot be empty".into());
        }
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("pane layout contains duplicate terminal IDs".into());
        }
        if ids.binary_search(&self.focused).is_err() {
            return Err("focused terminal is not present in pane layout".into());
        }
        validate_node(&self.root)
    }
}

fn validate_node(node: &PaneNode) -> Result<(), String> {
    match node {
        PaneNode::Terminal(_) => Ok(()),
        PaneNode::Split {
            ratio,
            first,
            second,
            ..
        } => {
            if !ratio.is_finite() || !(0.1..=0.9).contains(ratio) {
                return Err("pane split ratio must be finite and between 0.1 and 0.9".into());
            }
            validate_node(first)?;
            validate_node(second)
        }
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalPaneSnapshot {
    pub old_id: u64,
    pub term: String,
    pub cols: u32,
    pub rows: u32,
    pub charset: TerminalCharset,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalManagerSnapshot {
    pub version: u32,
    pub layout: PaneLayout,
    pub panes: Vec<TerminalPaneSnapshot>,
}

/// Owns multiple independent PTY channels over one authenticated SSH connection.
pub struct TerminalManager {
    sessions: BTreeMap<u64, TerminalSession>,
    screens: BTreeMap<u64, TerminalScreen>,
    options: BTreeMap<u64, (String, u32, u32)>,
    active: Option<u64>,
    layout: Option<PaneLayout>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            screens: BTreeMap::new(),
            options: BTreeMap::new(),
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
        self.options.insert(id, (term.to_owned(), cols, rows));
        self.screens.insert(
            id,
            TerminalScreen::new(
                rows.min(u16::MAX as u32) as u16,
                cols.min(u16::MAX as u32) as u16,
                10_000,
            ),
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
            self.options.remove(&new_id);
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
        if let Some(layout) = &mut self.layout
            && !layout.focus(terminal_id)
        {
            return false;
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
        if let Some((_, saved_cols, saved_rows)) = self.options.get_mut(&terminal_id) {
            *saved_cols = cols;
            *saved_rows = rows;
        }
        if let Some(screen) = self.screens.get_mut(&terminal_id) {
            screen.resize(
                rows.min(u16::MAX as u32) as u16,
                cols.min(u16::MAX as u32) as u16,
            );
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
        let close_result = session.close().await;
        self.screens.remove(&terminal_id);
        self.options.remove(&terminal_id);
        close_result?;
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

    pub fn snapshot(&self) -> Option<TerminalManagerSnapshot> {
        let layout = self.layout.clone()?;
        let panes = layout
            .terminal_ids()
            .into_iter()
            .map(|old_id| {
                let (term, cols, rows) = self
                    .options
                    .get(&old_id)
                    .cloned()
                    .unwrap_or_else(|| ("xterm-256color".into(), 80, 24));
                TerminalPaneSnapshot {
                    old_id,
                    term,
                    cols,
                    rows,
                    charset: self
                        .screens
                        .get(&old_id)
                        .map(TerminalScreen::charset)
                        .unwrap_or(TerminalCharset::Utf8),
                }
            })
            .collect();
        Some(TerminalManagerSnapshot {
            version: 1,
            layout,
            panes,
        })
    }

    /// Reopens all PTYs from a snapshot and restores the split tree with fresh channel IDs.
    /// On any failure, every PTY opened by this restore attempt is closed.
    pub async fn restore(
        &mut self,
        ssh: &SshClient,
        snapshot: &TerminalManagerSnapshot,
    ) -> Result<BTreeMap<u64, u64>, SshError> {
        if !self.is_empty() {
            return Err(SshError::Proxy(
                "terminal manager must be empty before restore".into(),
            ));
        }
        if snapshot.version != 1 || snapshot.layout.validate().is_err() {
            return Err(SshError::Proxy(
                "invalid or unsupported terminal snapshot".into(),
            ));
        }
        let mut expected = snapshot.layout.terminal_ids();
        expected.sort_unstable();
        let mut supplied: Vec<_> = snapshot.panes.iter().map(|pane| pane.old_id).collect();
        supplied.sort_unstable();
        if expected != supplied {
            return Err(SshError::Proxy(
                "snapshot pane list does not match its layout".into(),
            ));
        }

        let mut remap = BTreeMap::new();
        for pane in &snapshot.panes {
            match self.open(ssh, &pane.term, pane.cols, pane.rows).await {
                Ok(new_id) => {
                    remap.insert(pane.old_id, new_id);
                    self.set_charset(new_id, pane.charset);
                }
                Err(error) => {
                    self.close_all().await;
                    return Err(error);
                }
            }
        }
        let Some(root) = remap_node(&snapshot.layout.root, &remap) else {
            self.close_all().await;
            return Err(SshError::Proxy("cannot remap restored pane layout".into()));
        };
        let Some(focused) = remap.get(&snapshot.layout.focused).copied() else {
            self.close_all().await;
            return Err(SshError::Proxy("snapshot focus target is missing".into()));
        };
        self.layout = Some(PaneLayout { root, focused });
        self.active = Some(focused);
        Ok(remap)
    }

    pub async fn close_all(&mut self) {
        for (_, mut session) in std::mem::take(&mut self.sessions) {
            let _ = session.close().await;
        }
        self.screens.clear();
        self.options.clear();
        self.active = None;
        self.layout = None;
    }
}

fn remap_node(node: &PaneNode, remap: &BTreeMap<u64, u64>) -> Option<PaneNode> {
    match node {
        PaneNode::Terminal(id) => remap.get(id).copied().map(PaneNode::Terminal),
        PaneNode::Split {
            axis,
            ratio,
            first,
            second,
        } => Some(PaneNode::Split {
            axis: *axis,
            ratio: *ratio,
            first: Box::new(remap_node(first, remap)?),
            second: Box::new(remap_node(second, remap)?),
        }),
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
    fn pane_layout_json_round_trips_and_rejects_invalid_state() {
        let mut layout = PaneLayout::new(10);
        assert!(layout.split_focused(11, SplitAxis::Horizontal, 0.65));
        let json = layout.to_json().unwrap();
        assert_eq!(PaneLayout::from_json(&json).unwrap(), layout);

        let duplicate = r#"{"root":{"type":"split","data":{"axis":"horizontal","ratio":0.5,"first":{"type":"terminal","data":7},"second":{"type":"terminal","data":7}}},"focused":7}"#;
        assert!(PaneLayout::from_json(duplicate).is_err());
        let invalid_ratio = r#"{"root":{"type":"split","data":{"axis":"horizontal","ratio":1.5,"first":{"type":"terminal","data":7},"second":{"type":"terminal","data":8}}},"focused":7}"#;
        assert!(PaneLayout::from_json(invalid_ratio).is_err());
    }

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
