import ELK from 'elkjs/lib/elk.bundled.js';
import type { ElkNode } from 'elkjs';

const elk = new ELK();
self.onmessage = async (event: MessageEvent<ElkNode>) => {
  try {
    const graph = await elk.layout(event.data);
    self.postMessage({ positions: Object.fromEntries((graph.children ?? []).map(node => [node.id, { x: (node.x ?? 0) + (node.width ?? 0) / 2, y: (node.y ?? 0) + (node.height ?? 0) / 2 }])) });
  } catch (error) {
    self.postMessage({ error: error instanceof Error ? error.message : String(error) });
  }
};
