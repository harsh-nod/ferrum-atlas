import type { AnalysisEnvelope } from "../api/types";
import { CoverageNotice } from "./common";

export function AnalysisNotes({ envelope }: { envelope: AnalysisEnvelope }) {
  return (
    <>
      <CoverageNotice coverage={envelope.coverage} />
      {(envelope.truncated ||
        envelope.cancelled ||
        envelope.deadline_reached) && (
        <p className="notice" role="status">
          {envelope.cancelled
            ? "Cancelled"
            : envelope.deadline_reached
              ? "Deadline reached"
              : "Bounded result"}
        </p>
      )}
      <details className="analysis-assumptions">
        <summary>Algorithm and assumptions</summary>
        <p className="mono">{envelope.algorithm_version}</p>
        <ul>
          {envelope.assumptions.map((assumption, index) => (
            <li key={index}>{assumption}</li>
          ))}
        </ul>
      </details>
    </>
  );
}
