import type { ElementDefinition } from "cytoscape";
import type { GraphResponse, Relation } from "./api/types";

export function graphElements(graph: GraphResponse, edges: Relation[]) {
  const visible = graph.nodes.slice(0, 200);
  const ids = new Set(visible.map((node) => node.id));
  const elements: ElementDefinition[] = visible.map((node) => ({
    data: { id: node.id, label: node.name, fullLabel: node.qualified_name },
  }));
  let sites = 0;
  for (const edge of edges.slice(0, 500)) {
    if (!ids.has(edge.source)) continue;
    const target =
      edge.target.kind === "resolved" ? edge.target.id : `unknown:${edge.id}`;
    if (edge.target.kind === "unknown") {
      if (ids.size >= 200) continue;
      ids.add(target);
      elements.push({
        data: {
          id: target,
          label: `? ${edge.target.label}`,
          fullLabel: `${edge.target.label}: ${edge.target.reason.replaceAll("_", " ")}`,
        },
        classes: "unknown",
      });
    }
    if (!ids.has(target)) continue;
    elements.push({
      data: {
        id: edge.id,
        source: edge.source,
        target,
        label: edge.target.kind === "unknown" ? "unknown" : edge.kind,
      },
      classes: edge.target.kind === "unknown" ? "uncertain" : "",
    });
    sites++;
  }
  return { elements, nodes: ids.size, sites, omitted: edges.length - sites };
}
