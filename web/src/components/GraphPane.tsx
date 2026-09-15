import { useEffect, useMemo, useRef, useState } from "react";
import cytoscape from "cytoscape";
import ELK from "elkjs/lib/elk-api.js";
import LayoutWorker from "elkjs/lib/elk-worker.min.js?worker";
import type { Core } from "cytoscape";
import {
  Crosshair,
  GitFork,
  Minus,
  Plus,
  Table2,
  Workflow,
} from "lucide-react";
import type { GraphResponse, Relation } from "../api/types";
import { CoverageNotice, IconButton, Loading } from "./common";
import { graphElements } from "../graph-model";

type Positions = Record<string, { x: number; y: number }>;
const layouts = new Map<string, Positions>();

export function GraphPane({
  graph,
  selected,
  selectedEdge,
  onNode,
  onEdge,
}: {
  graph: GraphResponse;
  selected: string;
  selectedEdge: string;
  onNode: (id: string) => void;
  onEdge: (edge: Relation) => void;
}) {
  const mount = useRef<HTMLDivElement>(null);
  const instance = useRef<Core | null>(null);
  const callbacks = useRef({ onNode, onEdge });
  callbacks.current = { onNode, onEdge };
  const selection = useRef(selectedEdge || selected);
  selection.current = selectedEdge || selected;
  const [mode, setMode] = useState<"graph" | "table">("graph");
  const [vertical, setVertical] = useState(false);
  const [showUnknown, setShowUnknown] = useState(true);
  const [layoutState, setLayoutState] = useState("loading");
  const [layoutError, setLayoutError] = useState("");
  const [hover, setHover] = useState("");
  const edges = useMemo(
    () =>
      graph.edges.filter(
        (edge) => showUnknown || edge.target.kind === "resolved",
      ),
    [graph, showUnknown],
  );
  const nodes = useMemo(
    () => new Map(graph.nodes.map((node) => [node.id, node])),
    [graph],
  );
  const display = useMemo(() => graphElements(graph, edges), [graph, edges]);
  const key = JSON.stringify([
    vertical,
    graph.snapshot_id,
    graph.context_id,
    graph.nodes.map((node) => node.id),
    edges.map((edge) => edge.id),
  ]);

  useEffect(() => {
    if (!mount.current || mode !== "graph") return;
    let disposed = false;
    setLayoutState("loading");
    setLayoutError("");
    const elements = display.elements;
    const cy = cytoscape({
      container: mount.current,
      elements,
      layout: { name: "preset" },
      minZoom: 0.2,
      maxZoom: 2.5,
      style: [
        {
          selector: "node",
          style: {
            shape: "round-rectangle",
            width: 154,
            height: 46,
            "background-color": "#ffffff",
            "border-width": 1.5,
            "border-color": "#82a797",
            label: "data(label)",
            "font-size": 12,
            "font-family": "system-ui",
            "text-valign": "center",
            "text-halign": "center",
            color: "#243b31",
            "text-max-width": "138px",
            "text-wrap": "ellipsis",
          },
        },
        {
          selector: "node.unknown",
          style: {
            "border-style": "dashed",
            "border-color": "#a5782b",
            "background-color": "#fffaf0",
            color: "#755514",
          },
        },
        {
          selector: "node:selected",
          style: {
            "border-color": "#117d5e",
            "border-width": 3,
            "background-color": "#e4f3eb",
          },
        },
        {
          selector: "edge",
          style: {
            width: 1.8,
            "line-color": "#769a89",
            "target-arrow-color": "#769a89",
            "target-arrow-shape": "triangle",
            "curve-style": "bezier",
            label: "data(label)",
            "font-size": 10,
            color: "#627c6e",
            "text-background-color": "#f7faf8",
            "text-background-opacity": 1,
            "text-background-padding": "3px",
            "text-rotation": "autorotate",
          },
        },
        {
          selector: "edge.uncertain",
          style: {
            "line-style": "dashed",
            "line-color": "#aa853f",
            "target-arrow-color": "#aa853f",
          },
        },
        {
          selector: "edge:selected",
          style: {
            "line-color": "#167258",
            "target-arrow-color": "#167258",
            width: 3.5,
          },
        },
      ],
    });
    instance.current = cy;
    cy.on("tap", "node", (event) => {
      const id = event.target.id();
      if (nodes.has(id)) callbacks.current.onNode(id);
      else {
        const edge = edges.find((item) => `unknown:${item.id}` === id);
        if (edge) callbacks.current.onEdge(edge);
      }
    });
    cy.on("tap", "edge", (event) => {
      const edge = edges.find((item) => item.id === event.target.id());
      if (edge) callbacks.current.onEdge(edge);
    });
    cy.on("mouseover", "node", (event) =>
      setHover(event.target.data("fullLabel")),
    );
    cy.on("mouseout", "node", () => setHover(""));
    const resize = new ResizeObserver(() => {
      cy.resize();
      if (mount.current?.clientWidth && mount.current.clientHeight) {
        setVertical(
          mount.current.clientWidth < 600 &&
            mount.current.clientHeight > mount.current.clientWidth * 0.8,
        );
        cy.fit(undefined, 35);
      }
    });
    resize.observe(mount.current);
    const apply = (positions: Positions) => {
      if (disposed) return;
      cy.nodes().positions((node) => positions[node.id()] ?? { x: 0, y: 0 });
      cy.fit(undefined, 35);
      cy.elements().unselect();
      cy.getElementById(selection.current).select();
      setLayoutState("ready");
    };
    let worker: Worker | undefined;
    let layoutTimeout: ReturnType<typeof setTimeout> | undefined;
    const cached = layouts.get(key);
    if (cached) apply(cached);
    else {
      const fail = () => {
        clearTimeout(layoutTimeout);
        worker?.terminate();
        if (!disposed) {
          setLayoutError(
            "Graph layout unavailable. Relationships remain available in the table.",
          );
          setLayoutState("failed");
        }
      };
      const elk = new ELK({
        workerFactory: () => {
          worker = new LayoutWorker();
          worker.onerror = fail;
          return worker;
        },
      });
      layoutTimeout = setTimeout(fail, 5000);
      elk
        .layout({
          id: "root",
          layoutOptions: {
            "elk.algorithm": "layered",
            "elk.direction": vertical ? "DOWN" : "RIGHT",
            "elk.spacing.nodeNode": "35",
            "elk.layered.spacing.nodeNodeBetweenLayers": "85",
          },
          children: cy
            .nodes()
            .map((node) => ({ id: node.id(), width: 154, height: 46 })),
          edges: cy.edges().map((edge) => ({
            id: edge.id(),
            sources: [edge.source().id()],
            targets: [edge.target().id()],
          })),
        })
        .then((result) => {
          clearTimeout(layoutTimeout);
          if (disposed) return;
          const positions: Positions = Object.fromEntries(
            (result.children ?? []).map((node) => [
              node.id,
              {
                x: (node.x ?? 0) + (node.width ?? 0) / 2,
                y: (node.y ?? 0) + (node.height ?? 0) / 2,
              },
            ]),
          );
          layouts.set(key, positions);
          if (layouts.size > 20) layouts.delete(layouts.keys().next().value!);
          apply(positions);
        })
        .catch(fail);
    }
    return () => {
      disposed = true;
      clearTimeout(layoutTimeout);
      worker?.terminate();
      resize.disconnect();
      cy.destroy();
      instance.current = null;
    };
  }, [key, mode]);

  useEffect(() => {
    instance.current?.elements().unselect();
    instance.current?.getElementById(selectedEdge || selected).select();
  }, [selected, selectedEdge]);

  return (
    <section className="graph-pane" aria-label="Call neighborhood">
      <div className="panel-heading">
        <GitFork size={16} />
        <strong>Call neighborhood</strong>
        <span className="muted">
          {mode === "graph" ? display.nodes : graph.nodes.length} nodes ·{" "}
          {mode === "graph" ? display.sites : edges.length} sites
        </span>
        <div className="spacer" />
        <CoverageNotice coverage={graph.coverage} compact />
      </div>
      <div className="graph-toolbar">
        <div className="segmented">
          <IconButton
            label="Graph view"
            aria-pressed={mode === "graph"}
            onClick={() => setMode("graph")}
          >
            <Workflow size={16} />
          </IconButton>
          <IconButton
            label="Relationship table"
            aria-pressed={mode === "table"}
            onClick={() => setMode("table")}
          >
            <Table2 size={16} />
          </IconButton>
        </div>
        <label className="check-label">
          <input
            type="checkbox"
            checked={showUnknown}
            onChange={(event) => setShowUnknown(event.target.checked)}
          />
          Unknown targets
        </label>
        <div className="spacer" />
        <IconButton
          label="Zoom in"
          onClick={() => instance.current?.zoom(instance.current.zoom() * 1.2)}
        >
          <Plus size={15} />
        </IconButton>
        <IconButton
          label="Zoom out"
          onClick={() => instance.current?.zoom(instance.current.zoom() / 1.2)}
        >
          <Minus size={15} />
        </IconButton>
        <IconButton
          label="Fit graph"
          onClick={() => instance.current?.fit(undefined, 35)}
        >
          <Crosshair size={16} />
        </IconButton>
      </div>
      {mode === "graph" ? (
        <div className="graph-surface">
          <div
            ref={mount}
            className="graph-canvas"
            data-testid="graph-canvas"
            data-layout-state={layoutState}
            data-node-count={display.nodes}
            data-edge-count={display.sites}
            aria-label="Interactive call graph"
            role="img"
          />
          {layoutState === "loading" && (
            <div className="graph-overlay">
              <Loading label="Laying out graph" />
            </div>
          )}
          {layoutError && (
            <div className="notice error" role="alert">
              {layoutError}
            </div>
          )}
          {hover && (
            <div className="graph-tooltip" role="tooltip">
              {hover}
            </div>
          )}
          <div className="graph-legend">
            <span className="line solid" />
            Resolved <span className="line dashed" />
            Unknown
          </div>
        </div>
      ) : (
        <div className="relationship-table">
          <table>
            <thead>
              <tr>
                <th>Caller</th>
                <th>Target</th>
                <th>Evidence</th>
                <th>Source</th>
              </tr>
            </thead>
            <tbody>
              {edges.map((edge) => (
                <tr
                  key={edge.id}
                  className={edge.id === selectedEdge ? "selected" : ""}
                >
                  <td>
                    <button
                      className="text-button"
                      onClick={() => onNode(edge.source)}
                    >
                      {nodes.get(edge.source)?.name ?? edge.source}
                    </button>
                  </td>
                  <td>
                    {edge.target.kind === "resolved"
                      ? (nodes.get(edge.target.id)?.name ?? edge.target.id)
                      : `? ${edge.target.label}`}
                  </td>
                  <td>
                    {edge.target.kind === "resolved"
                      ? "Resolved"
                      : edge.target.reason.replaceAll("_", " ")}
                  </td>
                  <td>
                    <button
                      className="text-button"
                      onClick={() => onEdge(edge)}
                    >
                      Call site
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {edges.length === 0 && (
            <div className="empty">
              No relationships in this result. Analysis coverage:{" "}
              {graph.coverage.status}.
            </div>
          )}
        </div>
      )}
      {mode === "graph" && display.omitted > 0 && (
        <div className="window-notice">
          {display.omitted} sites omitted from the bounded canvas; available in
          the relationship table.
        </div>
      )}
      {(graph.page.truncated || graph.work.deadline_reached) && (
        <div className="window-notice">
          {graph.work.deadline_reached ? "Query deadline reached. " : ""}
          {graph.page.truncated
            ? "Result limited by graph budgets. Narrow the scope or depth."
            : ""}
        </div>
      )}
    </section>
  );
}
