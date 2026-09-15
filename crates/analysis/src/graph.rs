use crate::{AnalysisControl, AnalysisEnvelope, AnalysisError, Result, byte_budget};
use atlas_model::{
    CfgStatus, Coverage, Definition, DefinitionId, Relation, RelationId, Status, Target,
    UnknownReason, digest,
};
use petgraph::{Directed, Graph, algo::kosaraju_scc, graph::NodeIndex, visit::Bfs};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

pub struct SelectedGraph<'a> {
    pub nodes: &'a [Definition],
    pub edges: &'a [Relation],
    pub coverage: &'a Coverage,
    pub input_truncated: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
pub struct AnalysisLimits {
    pub max_nodes: u32,
    pub max_edges: u32,
    pub max_visits: u32,
    pub max_depth: u32,
    pub max_response_bytes: u32,
}
impl Default for AnalysisLimits {
    fn default() -> Self {
        Self {
            max_nodes: 200,
            max_edges: 500,
            max_visits: 200,
            max_depth: 8,
            max_response_bytes: 2 * 1024 * 1024,
        }
    }
}
impl AnalysisLimits {
    fn validate(self) -> Result<()> {
        if !(1..=10_000).contains(&self.max_nodes)
            || !(1..=50_000).contains(&self.max_edges)
            || !(1..=10_000).contains(&self.max_visits)
            || self.max_depth > 64
            || !(256..=2 * 1024 * 1024).contains(&self.max_response_bytes)
        {
            return Err(AnalysisError::InvalidInput("limits require 1..10000 nodes/visits, 1..50000 edges, depth <=64 and 256..2097152 response bytes".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct UnknownFrontier {
    pub source: DefinitionId,
    pub relation_id: RelationId,
    pub target_definition_id: Option<DefinitionId>,
    pub reason: UnknownReason,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RecursiveComponent {
    pub id: String,
    pub members: Vec<DefinitionId>,
    pub recursive: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ComponentLink {
    pub source: String,
    pub target: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DefinitionMetrics {
    pub definition_id: DefinitionId,
    pub distinct_callers: u32,
    pub distinct_callees: u32,
    pub incoming_call_sites: u32,
    pub outgoing_call_sites: u32,
    pub unknown_call_sites: u32,
    pub source_lines: u32,
    pub source_branches: u32,
    pub source_returns: u32,
    pub source_awaits: u32,
    pub source_unsafe_blocks: u32,
    pub declared_public: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct MetricDistribution {
    pub metric: String,
    pub minimum: u32,
    pub median: u32,
    pub p95: u32,
    pub maximum: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GraphAnalysis {
    pub envelope: AnalysisEnvelope,
    pub selected_nodes: u32,
    pub selected_call_sites: u32,
    pub components: Vec<RecursiveComponent>,
    pub component_links: Vec<ComponentLink>,
    pub metrics: Vec<DefinitionMetrics>,
    pub distributions: Vec<MetricDistribution>,
    pub unknown_frontier: Vec<UnknownFrontier>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PathOutcome {
    Found,
    NotFoundInSelection,
    ReachabilityComplete,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PathAnalysis {
    pub envelope: AnalysisEnvelope,
    pub outcome: PathOutcome,
    pub path: Vec<DefinitionId>,
    pub relations: Vec<RelationId>,
    pub reachable: Vec<DefinitionId>,
    pub unknown_frontier: Vec<UnknownFrontier>,
}

struct Prepared<'a> {
    graph: Graph<&'a Definition, &'a Relation, Directed>,
    indexes: BTreeMap<DefinitionId, NodeIndex>,
    envelope: AnalysisEnvelope,
    unknown: Vec<UnknownFrontier>,
    sites: Vec<&'a Relation>,
}

fn prepare<'a>(
    input: &SelectedGraph<'a>,
    limits: AnalysisLimits,
    control: &AnalysisControl,
    version: &str,
) -> Result<Prepared<'a>> {
    limits.validate()?;
    if input.nodes.len() > 10_000 || input.edges.len() > 50_000 {
        return Err(AnalysisError::BudgetExhausted);
    }
    byte_budget(
        &(input.nodes, input.edges, input.coverage),
        16 * 1024 * 1024,
    )?;
    let mut envelope = AnalysisEnvelope::new(version, input.coverage.clone());
    envelope.assumptions = vec![
        "Only selected direct calls are analyzed; selected-subgraph SCCs and fan counts do not describe the entire repository".into(),
        "Resolved static calls show possible call relations, not path feasibility or execution evidence".into(),
        "Source line/branch/return/await/unsafe counts retain frontend semantics; they are not compiler CFG complexity or noncomment LOC".into(),
    ];
    let mut graph = Graph::new();
    let mut indexes = BTreeMap::new();
    let mut nodes = BTreeMap::new();
    let mut context = None;
    for node in input.nodes {
        if control.stopped() {
            envelope.stop(control);
            break;
        }
        if context.as_ref().is_some_and(|id| id != &node.context_id) {
            return Err(AnalysisError::InvalidInput(
                "selected definitions mix build contexts".into(),
            ));
        }
        context = Some(node.context_id.clone());
        if nodes.insert(node.id.clone(), node).is_some() {
            return Err(AnalysisError::InvalidInput(
                "duplicate definition identity".into(),
            ));
        }
    }
    let mut edge_ids = BTreeSet::new();
    let mut edges = Vec::new();
    for edge in input.edges {
        if control.stopped() {
            envelope.stop(control);
            break;
        }
        if !edge_ids.insert(edge.id.clone()) {
            return Err(AnalysisError::InvalidInput(
                "duplicate relation identity".into(),
            ));
        }
        if edge.kind == "calls" {
            edges.push(edge);
        }
    }
    edges.sort_by(|a, b| a.id.cmp(&b.id));
    if input.input_truncated
        || nodes.len() > limits.max_nodes as usize
        || edges.len() > limits.max_edges as usize
    {
        envelope.truncated = true;
        envelope.partial(UnknownReason::BudgetExhausted, "Selected graph was truncated; omitted nodes or call sites can change reachability, SCCs and fan counts");
    }
    for (id, node) in nodes.into_iter().take(limits.max_nodes as usize) {
        if node.cfg_status != CfgStatus::Active {
            envelope.partial(
                UnknownReason::CfgUnknown,
                "Selected graph contains inactive or unresolved configuration definitions",
            );
        }
        indexes.insert(id, graph.add_node(node));
    }
    let mut unknown = Vec::new();
    let mut sites = Vec::new();
    // Reverse insertion makes petgraph's neighbor iterator deterministic by target then call-site ID.
    let mut retained = edges
        .into_iter()
        .take(limits.max_edges as usize)
        .collect::<Vec<_>>();
    retained.sort_by(|a, b| {
        let key = |edge: &Relation| match &edge.target {
            Target::Resolved { id } => id.0.clone(),
            Target::Unknown { .. } => String::new(),
        };
        (key(b), &b.id).cmp(&(key(a), &a.id))
    });
    for edge in retained {
        if control.stopped() {
            envelope.stop(control);
            break;
        }
        let Some(&source) = indexes.get(&edge.source) else {
            envelope.partial(
                UnknownReason::ExternalBoundary,
                "Call sites from outside the selected node set were omitted",
            );
            unknown.push(UnknownFrontier {
                source: edge.source.clone(),
                relation_id: edge.id.clone(),
                target_definition_id: match &edge.target {
                    Target::Resolved { id } => Some(id.clone()),
                    Target::Unknown { .. } => None,
                },
                reason: UnknownReason::ExternalBoundary,
            });
            continue;
        };
        sites.push(edge);
        let frontier = match &edge.target {
            Target::Resolved { id } => {
                if let Some(&target) = indexes.get(id) {
                    graph.add_edge(source, target, edge);
                    None
                } else {
                    Some(UnknownFrontier {
                        source: edge.source.clone(),
                        relation_id: edge.id.clone(),
                        target_definition_id: Some(id.clone()),
                        reason: UnknownReason::ExternalBoundary,
                    })
                }
            }
            Target::Unknown { reason, .. } => Some(UnknownFrontier {
                source: edge.source.clone(),
                relation_id: edge.id.clone(),
                target_definition_id: None,
                reason: reason.clone(),
            }),
        };
        if let Some(frontier) = frontier {
            envelope.partial(
                frontier.reason.clone(),
                "Unresolved or unselected call targets remain explicit unknown frontiers",
            );
            unknown.push(frontier);
        }
    }
    unknown.sort_by(|a, b| (&a.source, &a.relation_id).cmp(&(&b.source, &b.relation_id)));
    Ok(Prepared {
        graph,
        indexes,
        envelope,
        unknown,
        sites,
    })
}

pub fn analyze_graph(
    input: &SelectedGraph<'_>,
    limits: AnalysisLimits,
    control: &AnalysisControl,
) -> Result<GraphAnalysis> {
    let mut prepared = prepare(input, limits, control, "selected-call-graph-scc-metrics-v1")?;
    let mut components = Vec::new();
    let mut component_for = BTreeMap::new();
    if !control.stopped() {
        // Iterative Kosaraju is O(V+E); hard input caps bound its noninterruptible section.
        for component in kosaraju_scc(&prepared.graph) {
            if control.stopped() {
                prepared.envelope.stop(control);
                components.clear();
                component_for.clear();
                break;
            }
            let mut members = component
                .iter()
                .map(|&index| prepared.graph[index].id.clone())
                .collect::<Vec<_>>();
            members.sort();
            let recursive = component.len() > 1
                || prepared
                    .graph
                    .find_edge(component[0], component[0])
                    .is_some();
            let id = digest("selected-scc", &members);
            for member in &members {
                component_for.insert(member.clone(), id.clone());
            }
            components.push(RecursiveComponent {
                id,
                members,
                recursive,
            });
        }
    } else {
        prepared.envelope.stop(control);
    }
    components.sort_by(|a, b| a.members.cmp(&b.members));
    let mut links = BTreeSet::new();
    let mut callers: BTreeMap<DefinitionId, BTreeSet<DefinitionId>> = BTreeMap::new();
    let mut callees: BTreeMap<DefinitionId, BTreeSet<DefinitionId>> = BTreeMap::new();
    let mut incoming = BTreeMap::<DefinitionId, u32>::new();
    let mut outgoing = BTreeMap::<DefinitionId, u32>::new();
    let mut unknown_counts = BTreeMap::<DefinitionId, u32>::new();
    for frontier in &prepared.unknown {
        *unknown_counts.entry(frontier.source.clone()).or_default() += 1;
    }
    for edge in &prepared.sites {
        if control.stopped() {
            prepared.envelope.stop(control);
            break;
        }
        *outgoing.entry(edge.source.clone()).or_default() += 1;
        if let Target::Resolved { id } = &edge.target
            && prepared.indexes.contains_key(id)
        {
            *incoming.entry(id.clone()).or_default() += 1;
            callers
                .entry(id.clone())
                .or_default()
                .insert(edge.source.clone());
            callees
                .entry(edge.source.clone())
                .or_default()
                .insert(id.clone());
            if let (Some(source), Some(target)) =
                (component_for.get(&edge.source), component_for.get(id))
                && source != target
            {
                links.insert((source.clone(), target.clone()));
            }
        }
    }
    let mut metrics = Vec::new();
    for (id, &index) in &prepared.indexes {
        if control.stopped() {
            prepared.envelope.stop(control);
            break;
        }
        let node = prepared.graph[index];
        metrics.push(DefinitionMetrics {
            definition_id: id.clone(),
            distinct_callers: callers.get(id).map_or(0, |set| set.len() as u32),
            distinct_callees: callees.get(id).map_or(0, |set| set.len() as u32),
            incoming_call_sites: incoming.get(id).copied().unwrap_or(0),
            outgoing_call_sites: outgoing.get(id).copied().unwrap_or(0),
            unknown_call_sites: unknown_counts.get(id).copied().unwrap_or(0),
            source_lines: node.metrics.lines,
            source_branches: node.metrics.branches,
            source_returns: node.metrics.returns,
            source_awaits: node.metrics.awaits,
            source_unsafe_blocks: node.metrics.unsafe_blocks,
            declared_public: node.visibility == "pub",
        });
    }
    let distributions = if metrics.is_empty() {
        vec![]
    } else {
        vec![
            distribution("source_lines", metrics.iter().map(|m| m.source_lines)),
            distribution(
                "distinct_callers",
                metrics.iter().map(|m| m.distinct_callers),
            ),
            distribution(
                "distinct_callees",
                metrics.iter().map(|m| m.distinct_callees),
            ),
            distribution(
                "source_unsafe_blocks",
                metrics.iter().map(|m| m.source_unsafe_blocks),
            ),
        ]
    };
    let report = GraphAnalysis {
        envelope: prepared.envelope,
        selected_nodes: prepared.graph.node_count() as u32,
        selected_call_sites: prepared.sites.len() as u32,
        components,
        component_links: links
            .into_iter()
            .map(|(source, target)| ComponentLink { source, target })
            .collect(),
        metrics,
        distributions,
        unknown_frontier: prepared.unknown,
    };
    byte_budget(&report, limits.max_response_bytes as usize)?;
    Ok(report)
}

fn distribution(metric: &str, values: impl Iterator<Item = u32>) -> MetricDistribution {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable();
    MetricDistribution {
        metric: metric.into(),
        minimum: values[0],
        median: values[(values.len() - 1) / 2],
        p95: values[(values.len() * 95).div_ceil(100) - 1],
        maximum: *values.last().unwrap(),
    }
}

/// Finds one shortest outgoing static path, or explores reachability when target is absent.
pub fn find_path(
    input: &SelectedGraph<'_>,
    start: &DefinitionId,
    target: Option<&DefinitionId>,
    limits: AnalysisLimits,
    control: &AnalysisControl,
) -> Result<PathAnalysis> {
    let mut prepared = prepare(input, limits, control, "selected-call-graph-bfs-v1")?;
    let mut outcome = PathOutcome::Unknown;
    let mut visited = Vec::new();
    let mut path = Vec::new();
    let mut relations = Vec::new();
    let mut found = None;
    let mut previous = BTreeMap::<NodeIndex, (NodeIndex, RelationId)>::new();
    let mut distances = BTreeMap::<NodeIndex, u32>::new();
    if let Some(&root) = prepared.indexes.get(start) {
        let mut bfs = Bfs::new(&prepared.graph, root);
        distances.insert(root, 0);
        while let Some(index) = bfs.next(&prepared.graph) {
            if control.stopped() {
                prepared.envelope.stop(control);
                break;
            }
            let depth = distances[&index];
            if visited.len() >= limits.max_visits as usize || depth > limits.max_depth {
                prepared.envelope.truncated = true;
                prepared.envelope.partial(UnknownReason::BudgetExhausted, "Reachability visited-set or depth budget exhausted; no complete negative is available");
                break;
            }
            visited.push(prepared.graph[index].id.clone());
            if target.is_some_and(|target| target == &prepared.graph[index].id) {
                found = Some(index);
                outcome = PathOutcome::Found;
                break;
            }
            use petgraph::visit::EdgeRef;
            for edge in prepared.graph.edges(index) {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    distances.entry(edge.target())
                {
                    entry.insert(depth + 1);
                    previous.insert(edge.target(), (index, edge.weight().id.clone()));
                }
            }
        }
        if found.is_none()
            && !prepared.envelope.truncated
            && prepared.envelope.coverage.status == Status::Complete
            && target.is_none_or(|target| prepared.indexes.contains_key(target))
        {
            outcome = if target.is_some() {
                PathOutcome::NotFoundInSelection
            } else {
                PathOutcome::ReachabilityComplete
            };
        }
        if let Some(mut index) = found {
            path.push(prepared.graph[index].id.clone());
            while let Some((parent, relation)) = previous.get(&index) {
                relations.push(relation.clone());
                index = *parent;
                path.push(prepared.graph[index].id.clone());
            }
            path.reverse();
            relations.reverse();
        }
    } else if !prepared.envelope.truncated {
        return Err(AnalysisError::InvalidInput(
            "start definition is absent from selected graph".into(),
        ));
    }
    if target.is_some_and(|target| !prepared.indexes.contains_key(target)) {
        prepared.envelope.partial(UnknownReason::ExternalBoundary, "Requested target is outside the selected graph; absence cannot establish unreachability");
    }
    let reached = visited.iter().cloned().collect::<BTreeSet<_>>();
    let unknown_frontier = prepared
        .unknown
        .into_iter()
        .filter(|frontier| reached.contains(&frontier.source))
        .collect();
    let report = PathAnalysis {
        envelope: prepared.envelope,
        outcome,
        path,
        relations,
        reachable: visited,
        unknown_frontier,
    };
    byte_budget(&report, limits.max_response_bytes as usize)?;
    Ok(report)
}
