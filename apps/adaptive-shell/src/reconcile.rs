//! Keyed reconciliation over the semantic scene graph.

use crate::components::{NodeId, SceneError, SemanticNode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A retained node record.  Renderer handles intentionally do not live here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetainedNode {
    /// Stable semantic identity.
    pub id: NodeId,
    /// Semantic kind at the last reconciliation.
    pub kind: crate::components::ComponentKind,
    /// Parent identity, if any.
    pub parent: Option<NodeId>,
    /// Number of times this identity has been reconciled.
    pub generation: u64,
}

/// Keyed reconciliation outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReconcileReport {
    /// Nodes whose identity and kind were reused.
    pub reused: Vec<NodeId>,
    /// Newly materialized semantic nodes.
    pub created: Vec<NodeId>,
    /// Identities removed from the retained tree.
    pub removed: Vec<NodeId>,
    /// Focus survived the update.
    pub focus_preserved: bool,
}

/// Retained semantic tree with stable focus.
#[derive(Clone, Debug, Default)]
pub struct Reconciler {
    nodes: BTreeMap<NodeId, RetainedNode>,
    focus: Option<NodeId>,
}

impl Reconciler {
    /// Create an empty reconciler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconcile a new semantic root using stable keyed identities.
    pub fn reconcile(&mut self, root: &SemanticNode) -> Result<ReconcileReport, SceneError> {
        root.validate()?;
        let mut next = BTreeMap::new();
        collect(root, None, &self.nodes, &mut next);
        let mut reused = Vec::new();
        let mut created = Vec::new();
        for (id, node) in &next {
            if self
                .nodes
                .get(id)
                .is_some_and(|previous| previous.kind == node.kind)
            {
                reused.push(*id);
            } else {
                created.push(*id);
            }
        }
        let mut removed = self
            .nodes
            .keys()
            .filter(|id| !next.contains_key(id))
            .copied()
            .collect::<Vec<_>>();
        reused.sort_unstable();
        created.sort_unstable();
        removed.sort_unstable();
        let focus_preserved = self.focus.is_some_and(|focus| next.contains_key(&focus));
        if !focus_preserved {
            self.focus = next.keys().next().copied();
        }
        self.nodes = next;
        Ok(ReconcileReport {
            reused,
            created,
            removed,
            focus_preserved,
        })
    }

    /// Set focus if the identity is currently present.
    pub fn focus(&mut self, id: NodeId) -> bool {
        if self.nodes.contains_key(&id) {
            self.focus = Some(id);
            true
        } else {
            false
        }
    }

    /// Return the currently focused identity.
    pub const fn focused(&self) -> Option<NodeId> {
        self.focus
    }

    /// Clear focus (for a transient modal or launcher).
    pub const fn clear_focus(&mut self) {
        self.focus = None;
    }

    /// Return a retained node by identity.
    pub fn get(&self, id: NodeId) -> Option<&RetainedNode> {
        self.nodes.get(&id)
    }

    /// Return identities in deterministic order.
    pub fn ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.keys().copied()
    }
}

fn collect(
    node: &SemanticNode,
    parent: Option<NodeId>,
    previous: &BTreeMap<NodeId, RetainedNode>,
    output: &mut BTreeMap<NodeId, RetainedNode>,
) {
    let generation = previous
        .get(&node.id)
        .map_or(1, |retained| retained.generation.saturating_add(1));
    output.insert(
        node.id,
        RetainedNode {
            id: node.id,
            kind: node.kind,
            parent,
            generation,
        },
    );
    for child in &node.children {
        collect(child, Some(node.id), previous, output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{ComponentKind, SemanticNode};

    #[test]
    fn keyed_reconcile_reuses_identity_and_focus() {
        let mut root = SemanticNode::new("root", ComponentKind::Region, "Root");
        let mut button = SemanticNode::new("stable", ComponentKind::Button, "Stable");
        button.props = button
            .props
            .clone()
            .with(
                "label",
                crate::components::PropValue::Text("Stable".to_owned()),
            )
            .expect("property");
        button.props = button
            .props
            .clone()
            .with(
                "action",
                crate::components::PropValue::Text("noop".to_owned()),
            )
            .expect("property");
        root.push(button).expect("child");
        let stable = root.children[0].id;
        let mut reconciler = Reconciler::new();
        let first = reconciler.reconcile(&root).expect("valid");
        assert_eq!(first.created.len(), 2);
        assert!(reconciler.focus(stable));
        root.children[0].accessibility.name = "Still stable".to_owned();
        let second = reconciler.reconcile(&root).expect("valid");
        assert!(second.focus_preserved);
        assert!(second.reused.contains(&stable));
        assert_eq!(reconciler.focused(), Some(stable));
    }
}
