//! Read-only graph helpers over a [`Flow`].

use super::{Flow, Node};

impl Flow {
    /// The node with this id, if any.
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// The trigger nodes, in `nodes` order.
    pub fn triggers(&self) -> Vec<&Node> {
        self.nodes.iter().filter(|n| n.kind.is_trigger()).collect()
    }

    /// The nodes reached by edges leaving `node`, in edge order. `port` must match the edge's
    /// port exactly: `None` selects edges without a port.
    pub fn successors(&self, node: &str, port: Option<&str>) -> Vec<&Node> {
        self.edges
            .iter()
            .filter(|e| e.from == node && e.port.as_deref() == port)
            .filter_map(|e| self.node(&e.to))
            .collect()
    }

    /// Canvas columns of node ids: a node's column is its longest path from any trigger (triggers
    /// are column 0), and nodes no trigger reaches share one final extra column. Rows follow the
    /// order of `nodes`. Meant for valid flows; on an invalid one (a cycle) it still terminates
    /// because relaxation is bounded by the node count.
    pub fn layers(&self) -> Vec<Vec<String>> {
        let mut depth: Vec<Option<usize>> = self
            .nodes
            .iter()
            .map(|n| n.kind.is_trigger().then_some(0))
            .collect();
        let index = |id: &str| self.nodes.iter().position(|n| n.id == id);
        for _ in 0..self.nodes.len() {
            let mut changed = false;
            for edge in &self.edges {
                let (Some(from), Some(to)) = (index(&edge.from), index(&edge.to)) else {
                    continue;
                };
                let Some(d) = depth[from] else { continue };
                // Never deeper than the node count, so a cycle cannot grow without bound.
                if depth[to].is_none_or(|old| old < d + 1) && d < self.nodes.len() {
                    depth[to] = Some(d + 1);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let reached = depth.iter().flatten().max().map_or(0, |m| m + 1);
        let orphans = depth.iter().any(Option::is_none);
        let mut columns = vec![Vec::new(); reached + usize::from(orphans)];
        for (node, d) in self.nodes.iter().zip(&depth) {
            columns[d.unwrap_or(reached)].push(node.id.clone());
        }
        columns
    }
}

#[cfg(test)]
mod tests {
    use crate::flow::{Edge, Node, NodeKind, TriggerStateParams, reference_flow};

    fn ids(nodes: Vec<&Node>) -> Vec<&str> {
        nodes.into_iter().map(|n| n.id.as_str()).collect()
    }

    #[test]
    fn successors_follow_edge_order_and_port() {
        let f = reference_flow();
        assert_eq!(ids(f.successors("moving", Some("true"))), ["on"]);
        assert_eq!(ids(f.successors("moving", Some("false"))), ["wait"]);
        assert_eq!(ids(f.successors("motion", None)), ["moving"]);
        assert!(f.successors("moving", None).is_empty());
        assert_eq!(f.triggers().len(), 1);
        assert!(f.node("nope").is_none());
    }

    #[test]
    fn layers_of_reference_flow() {
        assert_eq!(
            reference_flow().layers(),
            [
                vec!["motion"],
                vec!["moving"],
                vec!["on", "wait"],
                vec!["off"]
            ]
        );
    }

    #[test]
    fn three_triggers_share_a_layer_and_orphans_go_last() {
        let mut f = reference_flow();
        for id in ["m2", "m3"] {
            f.nodes.push(Node {
                id: id.into(),
                name: None,
                position: None,
                kind: NodeKind::TriggerState(TriggerStateParams::default()),
                extra: Default::default(),
            });
            f.edges.push(Edge {
                from: id.into(),
                to: "moving".into(),
                port: None,
            });
        }
        let mut lonely = f.nodes[4].clone();
        lonely.id = "lonely".into();
        f.nodes.push(lonely);
        let layers = f.layers();
        assert_eq!(layers[0], ["motion", "m2", "m3"]);
        assert_eq!(layers.last().unwrap(), &["lonely"]);
        assert_eq!(layers.len(), 5);
    }

    #[test]
    fn layers_terminate_on_a_cycle() {
        let mut f = reference_flow();
        f.edges.push(Edge {
            from: "off".into(),
            to: "wait".into(),
            port: None,
        });
        assert!(!f.layers().is_empty());
    }
}
