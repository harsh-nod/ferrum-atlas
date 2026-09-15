import { useEffect, useRef, useState } from "react";
import { Check, FileCode2, GitBranch, X } from "lucide-react";
import type {
  AnalysisResponse,
  Definition,
  Snapshot,
  Span,
  StateMachineInference,
  StateMachineSelection,
  StateTransitionDecision,
  StateTransitionReview,
} from "../api/types";
import { params, request } from "../api/client";
import { useResource } from "../state";
import { AnalysisNotes } from "./AnalysisViews";
import { ErrorNotice, Loading } from "./common";

type Selection = StateMachineSelection;

export function StateMachineView({
  snapshot,
  definition,
  onSpan,
}: {
  snapshot: Snapshot;
  definition: Definition;
  onSpan: (span: Span) => void;
}) {
  const [enumPath, setEnumPath] = useState("");
  const [place, setPlace] = useState("");
  const [selection, setSelection] = useState<Selection>();
  const [attempt, setAttempt] = useState(0);
  const [reviewer, setReviewer] = useState("");
  const [note, setNote] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<Error>();
  const [saved, setSaved] = useState<StateTransitionReview[]>([]);
  const pending = useRef<AbortController | undefined>(undefined);
  useEffect(() => () => pending.current?.abort(), []);
  const base = `/analysis/state-machine/${encodeURIComponent(definition.id)}`;
  const result = useResource<AnalysisResponse<StateMachineInference>>(
    selection ? JSON.stringify(selection) + attempt : "",
    (signal) => request(base, signal, selection),
  );
  const inference = result.data?.analysis;
  const reviewable =
    inference &&
    !inference.envelope.truncated &&
    !inference.envelope.cancelled &&
    !inference.envelope.deadline_reached;
  const reviews = useResource<StateTransitionReview[]>(
    reviewable ? inference.input_digest + attempt : "",
    (signal) => request(base + "/reviews" + params(selection!), signal),
  );
  async function review(candidate: string, decision: StateTransitionDecision) {
    if (!inference || !selection || !reviewable || saving) return;
    const controller = new AbortController();
    pending.current = controller;
    setSaving(true);
    setSaveError(undefined);
    const declaration: StateTransitionReview = {
      input_digest: inference.input_digest,
      candidate_id: candidate,
      reviewer: reviewer.trim(),
      note: note.trim(),
      decision,
    };
    try {
      const recorded = await request<StateTransitionReview>(
        base + "/reviews",
        controller.signal,
        { selection, review: declaration },
      );
      if (
        recorded.input_digest !== declaration.input_digest ||
        recorded.candidate_id !== declaration.candidate_id ||
        recorded.decision !== declaration.decision ||
        recorded.reviewer !== declaration.reviewer ||
        recorded.note !== declaration.note
      )
        throw new Error(
          "Review acknowledgement did not match the selected candidate",
        );
      if (!controller.signal.aborted)
        setSaved((previous) => [...previous, recorded]);
    } catch (error) {
      if (!controller.signal.aborted)
        setSaveError(error instanceof Error ? error : new Error(String(error)));
    } finally {
      if (!controller.signal.aborted) setSaving(false);
    }
  }
  const declarations = [...(reviews.data ?? []), ...saved].filter(
    (value, index, all) =>
      all.findIndex(
        (other) =>
          other.candidate_id === value.candidate_id &&
          other.input_digest === value.input_digest &&
          other.decision === value.decision &&
          other.reviewer === value.reviewer &&
          other.note === value.note,
      ) === index,
  );
  return (
    <section
      className="document-view state-machine-view"
      aria-label="State transition review"
    >
      <h3>State Transition Candidates</h3>
      <form
        className="analysis-controls"
        onSubmit={(event) => {
          event.preventDefault();
          pending.current?.abort();
          setSaving(false);
          setSaveError(undefined);
          setSaved([]);
          setSelection({
            snapshot_id: snapshot.id,
            context_id: snapshot.context.id,
            enum_path: enumPath.trim(),
            state_place: place.trim(),
          });
          setAttempt((value) => value + 1);
        }}
      >
        <label>
          Enum path
          <input
            required
            maxLength={128}
            value={enumPath}
            onChange={(event) => setEnumPath(event.target.value)}
          />
        </label>
        <label>
          State variable
          <input
            required
            maxLength={128}
            value={place}
            onChange={(event) => setPlace(event.target.value)}
          />
        </label>
        <button className="text-button" type="submit" disabled={result.loading}>
          <GitBranch size={15} /> Analyze states
        </button>
      </form>
      <ErrorNotice error={result.error} />
      {result.loading && (
        <Loading label="Inferring selected state transitions" />
      )}
      {inference && (
        <>
          <p className="phase-label">
            Syntactic candidates / {inference.enum_path} /{" "}
            {inference.state_place}
          </p>
          <AnalysisNotes envelope={inference.envelope} />
          {!reviewable && (
            <p className="notice">
              Review unavailable for a cancelled or truncated inference.
            </p>
          )}
          {inference.candidates.length === 0 && (
            <p className="muted">
              No supported transition candidates returned. Other transitions may
              exist.
            </p>
          )}
          {inference.candidates.length > 0 && (
            <>
              <div className="analysis-controls">
                <label>
                  Declared reviewer
                  <input
                    maxLength={128}
                    value={reviewer}
                    onChange={(event) => setReviewer(event.target.value)}
                  />
                </label>
                <label>
                  Review note
                  <input
                    maxLength={2048}
                    value={note}
                    onChange={(event) => setNote(event.target.value)}
                  />
                </label>
              </div>
              <ErrorNotice error={reviews.error ?? saveError} />
              {reviews.loading && (
                <Loading label="Loading reviewer declarations" />
              )}
              <div className="analysis-table-scroll">
                <table className="analysis-table">
                  <thead>
                    <tr>
                      <th>Transition</th>
                      <th>Guard / action</th>
                      <th>Review declarations</th>
                    </tr>
                  </thead>
                  <tbody>
                    {inference.candidates.map((candidate) => (
                      <tr key={candidate.id}>
                        <td data-label="Transition">
                          <button
                            type="button"
                            className="analysis-source"
                            onClick={() => onSpan(candidate.assignment_span)}
                            title="Open transition assignment"
                          >
                            <FileCode2 size={14} />
                            {candidate.from_variant} to {candidate.to_variant}
                          </button>
                        </td>
                        <td data-label="Guard / action">
                          <code>
                            {candidate.guard?.text ?? "No explicit arm guard"}
                          </code>
                          <details>
                            <summary>Action source</summary>
                            <pre>{candidate.action.text}</pre>
                          </details>
                        </td>
                        <td data-label="Review declarations">
                          {declarations
                            .filter(
                              (value) => value.candidate_id === candidate.id,
                            )
                            .map((value, index) => (
                              <p key={index}>
                                {value.decision} / {value.reviewer}
                                {value.note ? `: ${value.note}` : ""}
                              </p>
                            ))}
                          <div className="state-review-actions">
                            <button
                              type="button"
                              className="text-button"
                              disabled={
                                !reviewable ||
                                !reviewer.trim() ||
                                saving ||
                                reviews.loading ||
                                !!reviews.error
                              }
                              onClick={() =>
                                void review(candidate.id, "accepted")
                              }
                            >
                              <Check size={14} /> Accept candidate
                            </button>
                            <button
                              type="button"
                              className="text-button"
                              disabled={
                                !reviewable ||
                                !reviewer.trim() ||
                                saving ||
                                reviews.loading ||
                                !!reviews.error
                              }
                              onClick={() =>
                                void review(candidate.id, "rejected")
                              }
                            >
                              <X size={14} /> Reject candidate
                            </button>
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </>
          )}
          {inference.unknowns.length > 0 && (
            <details>
              <summary>
                Unknown boundaries ({inference.unknowns.length} returned)
              </summary>
              <ul>
                {inference.unknowns.map((unknown, index) => (
                  <li key={index}>
                    <button
                      type="button"
                      className="analysis-source"
                      onClick={() => onSpan(unknown.span)}
                    >
                      <FileCode2 size={14} />
                      {unknown.reason}
                    </button>
                  </li>
                ))}
              </ul>
            </details>
          )}
        </>
      )}
    </section>
  );
}
