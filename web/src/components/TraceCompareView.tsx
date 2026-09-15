import { useState } from "react";
import { GitCompareArrows } from "lucide-react";
import type {
  ObservationSummary,
  ObservationWindow,
  Snapshot,
  TraceAlignmentRequest,
  TraceAlignmentResponse,
  TraceComparison,
} from "../api/types";
import { params, request } from "../api/client";
import { useResource } from "../state";
import { ErrorNotice, Loading } from "./common";
import { AnalysisNotes } from "./AnalysisViews";

export function TraceCompareView({
  snapshot,
  snapshots,
}: {
  snapshot: Snapshot;
  snapshots: Snapshot[];
}) {
  const choices = snapshots.filter(
    (item) => item.repository_id === snapshot.repository_id,
  );
  const [beforeSnapshotId, setBeforeSnapshotId] = useState("");
  const beforeSnapshot =
    choices.find((item) => item.id === beforeSnapshotId) ??
    choices.find((item) => item.id !== snapshot.id) ??
    snapshot;
  const [beforeId, setBeforeId] = useState("");
  const [afterId, setAfterId] = useState("");
  const [beforeStream, setBeforeStream] = useState("");
  const [afterStream, setAfterStream] = useState("");
  const [beforeOffset, setBeforeOffset] = useState(0);
  const [afterOffset, setAfterOffset] = useState(0);
  const [maxEvents, setMaxEvents] = useState(200);
  const [submitted, setSubmitted] = useState<TraceAlignmentRequest | null>(
    null,
  );
  const beforePin = {
    snapshot_id: beforeSnapshot.id,
    context_id: beforeSnapshot.context.id,
  };
  const afterPin = {
    snapshot_id: snapshot.id,
    context_id: snapshot.context.id,
  };
  const beforeSummaries = useResource<ObservationSummary[]>(
    beforeSnapshot.id + ":trace-choices",
    (signal) => request("/observations" + params(beforePin), signal),
  );
  const afterSummaries = useResource<ObservationSummary[]>(
    snapshot.id + ":trace-choices",
    (signal) => request("/observations" + params(afterPin), signal),
  );
  const beforeObservation =
    beforeSummaries.data?.find(
      (item) => item.id === beforeId && item.event_count > 0,
    ) ?? beforeSummaries.data?.find((item) => item.event_count > 0);
  const afterObservation =
    afterSummaries.data?.find(
      (item) => item.id === afterId && item.event_count > 0,
    ) ?? afterSummaries.data?.find((item) => item.event_count > 0);
  const beforeWindow = useResource<ObservationWindow>(
    beforeObservation
      ? beforeSnapshot.id + beforeObservation.id + ":stream-choices"
      : "",
    (signal) =>
      request(
        `/observations/${encodeURIComponent(beforeObservation!.id)}` +
          params({ ...beforePin, limit: 200 }),
        signal,
      ),
  );
  const afterWindow = useResource<ObservationWindow>(
    afterObservation
      ? snapshot.id + afterObservation.id + ":stream-choices"
      : "",
    (signal) =>
      request(
        `/observations/${encodeURIComponent(afterObservation!.id)}` +
          params({ ...afterPin, limit: 200 }),
        signal,
      ),
  );
  const selectedBeforeStream =
    beforeStream || beforeWindow.data?.streams[0]?.id || "";
  const selectedAfterStream =
    afterStream || afterWindow.data?.streams[0]?.id || "";
  const comparison = useResource<TraceAlignmentResponse<TraceComparison>>(
    submitted ? JSON.stringify(submitted) : "",
    (signal) => request("/traces/compare", signal, submitted),
  );
  const reset = () => setSubmitted(null);
  const result = comparison.data?.comparison;
  return (
    <section className="unframed-section trace-compare-view">
      <h3>Compare Trace Windows</h3>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (
            beforeObservation &&
            afterObservation &&
            selectedBeforeStream &&
            selectedAfterStream
          )
            setSubmitted({
              before_snapshot_id: beforeSnapshot.id,
              before_context_id: beforeSnapshot.context.id,
              before_observation_id: beforeObservation.id,
              after_snapshot_id: snapshot.id,
              after_context_id: snapshot.context.id,
              after_observation_id: afterObservation.id,
              before_stream_id: selectedBeforeStream,
              after_stream_id: selectedAfterStream,
              before_offset: beforeOffset,
              after_offset: afterOffset,
              max_events: maxEvents,
              max_anchors: maxEvents,
            });
        }}
      >
        <div className="trace-comparison-columns">
          <fieldset>
            <legend>Before</legend>
            <label>
              Snapshot
              <select
                aria-label="Trace baseline snapshot"
                value={beforeSnapshot.id}
                onChange={(event) => {
                  setBeforeSnapshotId(event.target.value);
                  setBeforeId("");
                  setBeforeStream("");
                  reset();
                }}
              >
                {choices.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.revision.slice(0, 14)} / {item.context.name}
                  </option>
                ))}
              </select>
            </label>
            <ErrorNotice error={beforeSummaries.error} />
            <label>
              Observation
              <select
                aria-label="Before trace observation"
                value={beforeObservation?.id ?? ""}
                onChange={(event) => {
                  setBeforeId(event.target.value);
                  setBeforeStream("");
                  reset();
                }}
              >
                {!beforeObservation && (
                  <option value="">No trace observations</option>
                )}
                {beforeSummaries.data
                  ?.filter((item) => item.event_count > 0)
                  .map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.artifact.producer} / {item.event_count} events
                    </option>
                  ))}
              </select>
            </label>
            <ErrorNotice error={beforeWindow.error} />
            <label>
              Stream
              <input
                aria-label="Before trace stream"
                list="before-trace-streams"
                value={selectedBeforeStream}
                maxLength={256}
                required
                onChange={(event) => {
                  setBeforeStream(event.target.value);
                  reset();
                }}
              />
            </label>
            <datalist id="before-trace-streams">
              {beforeWindow.data?.streams.map((stream) => (
                <option key={stream.id} value={stream.id}>
                  {stream.clock_domain} / {stream.timestamp_unit}
                </option>
              ))}
            </datalist>
            <label>
              Event offset
              <input
                aria-label="Before event offset"
                type="number"
                min={0}
                max={10000}
                value={beforeOffset}
                required
                onChange={(event) => {
                  setBeforeOffset(Number(event.target.value));
                  reset();
                }}
              />
            </label>
          </fieldset>
          <fieldset>
            <legend>After</legend>
            <label>
              Snapshot
              <input
                value={`${snapshot.revision.slice(0, 14)} / ${snapshot.context.name}`}
                readOnly
                aria-label="After trace snapshot"
              />
            </label>
            <ErrorNotice error={afterSummaries.error} />
            <label>
              Observation
              <select
                aria-label="After trace observation"
                value={afterObservation?.id ?? ""}
                onChange={(event) => {
                  setAfterId(event.target.value);
                  setAfterStream("");
                  reset();
                }}
              >
                {!afterObservation && (
                  <option value="">No trace observations</option>
                )}
                {afterSummaries.data
                  ?.filter((item) => item.event_count > 0)
                  .map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.artifact.producer} / {item.event_count} events
                    </option>
                  ))}
              </select>
            </label>
            <ErrorNotice error={afterWindow.error} />
            <label>
              Stream
              <input
                aria-label="After trace stream"
                list="after-trace-streams"
                value={selectedAfterStream}
                maxLength={256}
                required
                onChange={(event) => {
                  setAfterStream(event.target.value);
                  reset();
                }}
              />
            </label>
            <datalist id="after-trace-streams">
              {afterWindow.data?.streams.map((stream) => (
                <option key={stream.id} value={stream.id}>
                  {stream.clock_domain} / {stream.timestamp_unit}
                </option>
              ))}
            </datalist>
            <label>
              Event offset
              <input
                aria-label="After event offset"
                type="number"
                min={0}
                max={10000}
                value={afterOffset}
                required
                onChange={(event) => {
                  setAfterOffset(Number(event.target.value));
                  reset();
                }}
              />
            </label>
          </fieldset>
        </div>
        <div className="analysis-controls">
          <label>
            Window size
            <input
              aria-label="Trace comparison window size"
              type="number"
              min={1}
              max={2000}
              value={maxEvents}
              required
              onChange={(event) => {
                setMaxEvents(Number(event.target.value));
                reset();
              }}
            />
          </label>
          <button
            className="text-button"
            type="submit"
            disabled={
              !beforeObservation ||
              !afterObservation ||
              !selectedBeforeStream ||
              !selectedAfterStream ||
              comparison.loading
            }
          >
            <GitCompareArrows size={15} />
            Compare traces
          </button>
        </div>
      </form>
      {comparison.loading && <Loading label="Comparing trace windows" />}
      <ErrorNotice error={comparison.error} />
      {result && (
        <div className="trace-comparison-result">
          <h4>
            {result.first_observed_divergence
              ? "First Observed Divergence"
              : result.anchor
                ? "No Divergence in Compared Prefix"
                : "Alignment Unavailable"}
          </h4>
          <dl className="context-list">
            <dt>Anchor</dt>
            <dd className="mono">{result.anchor ?? "None"}</dd>
            <dt>Compared events</dt>
            <dd>{result.compared_events}</dd>
            <dt>Matching prefix</dt>
            <dd>{result.matching_prefix_events}</dd>
          </dl>
          <div className="trace-comparison-columns">
            <div>
              <h4>Before</h4>
              <p className="mono artifact-digest">
                {result.before_artifact.sha256}
              </p>
              <p>
                {result.before_clock_domain} / {result.before_timestamp_unit}
              </p>
              <p>
                Lost events:{" "}
                <span className="mono">{result.before_loss_count}</span>
              </p>
              <p>
                Interval:{" "}
                <span className="mono">
                  {result.before_interval ?? "Unavailable"}
                </span>
              </p>
            </div>
            <div>
              <h4>After</h4>
              <p className="mono artifact-digest">
                {result.after_artifact.sha256}
              </p>
              <p>
                {result.after_clock_domain} / {result.after_timestamp_unit}
              </p>
              <p>
                Lost events:{" "}
                <span className="mono">{result.after_loss_count}</span>
              </p>
              <p>
                Interval:{" "}
                <span className="mono">
                  {result.after_interval ?? "Unavailable"}
                </span>
              </p>
            </div>
          </div>
          {result.first_observed_divergence && (
            <div className="trace-divergence">
              <p>
                {result.first_observed_divergence.reason.replaceAll("_", " ")}
              </p>
              <div className="trace-comparison-columns">
                <div>
                  <strong>
                    {result.first_observed_divergence.before_kind ??
                      "No event in selected window"}
                  </strong>
                  <p className="mono">
                    {result.first_observed_divergence.before?.sequence}
                  </p>
                </div>
                <div>
                  <strong>
                    {result.first_observed_divergence.after_kind ??
                      "No event in selected window"}
                  </strong>
                  <p className="mono">
                    {result.first_observed_divergence.after?.sequence}
                  </p>
                </div>
              </div>
            </div>
          )}
          {result.ambiguous_anchors.length > 0 && (
            <details>
              <summary>
                Ambiguous anchors ({result.ambiguous_anchors.length})
              </summary>
              <ul className="mono">
                {result.ambiguous_anchors.map((anchor) => (
                  <li key={anchor}>{anchor}</li>
                ))}
              </ul>
            </details>
          )}
          <AnalysisNotes envelope={result.envelope} />
        </div>
      )}
    </section>
  );
}
