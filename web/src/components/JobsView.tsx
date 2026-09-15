import { useEffect, useRef, useState } from "react";
import { Play, RefreshCw, Square } from "lucide-react";
import type { JobLevel, JobPriority, JobRecord, Snapshot } from "../api/types";
import { RequestError, request } from "../api/client";
import { ErrorNotice, IconButton, Loading } from "./common";

export function JobsView({ snapshot }: { snapshot: Snapshot }) {
  const [jobs, setJobs] = useState<JobRecord[] | null>(null);
  const [error, setError] = useState<Error>();
  const [actionError, setActionError] = useState<Error>();
  const [refresh, setRefresh] = useState(0);
  const [pending, setPending] = useState(false);
  const profile = snapshot.context.name;
  const [level, setLevel] = useState<JobLevel>("semantic");
  const [priority, setPriority] = useState<JobPriority>("workspace");
  const actionController = useRef<AbortController | null>(null);
  useEffect(() => () => actionController.current?.abort(), []);
  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = async () => {
      let disabled = false;
      try {
        const next = await request<JobRecord[]>("/jobs", controller.signal);
        if (!controller.signal.aborted) {
          setJobs(next);
          setError(undefined);
        }
      } catch (cause) {
        if (!controller.signal.aborted) {
          setError(cause instanceof Error ? cause : new Error(String(cause)));
          disabled =
            cause instanceof RequestError &&
            cause.code === "unsupported_capability";
        }
      }
      if (!controller.signal.aborted && !disabled)
        timer = setTimeout(load, 2000);
    };
    void load();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [refresh]);
  const disabled =
    error instanceof RequestError && error.code === "unsupported_capability";
  const mutate = async (path: string, body: unknown) => {
    actionController.current?.abort();
    const controller = new AbortController();
    actionController.current = controller;
    setPending(true);
    setActionError(undefined);
    try {
      const record = await request<JobRecord>(path, controller.signal, body);
      if (!controller.signal.aborted) {
        setJobs((previous) => [
          record,
          ...(previous ?? []).filter((job) => job.id !== record.id),
        ]);
        setRefresh((value) => value + 1);
      }
    } catch (cause) {
      if (!controller.signal.aborted)
        setActionError(
          cause instanceof Error ? cause : new Error(String(cause)),
        );
    } finally {
      if (!controller.signal.aborted) setPending(false);
    }
  };
  return (
    <section className="unframed-section jobs-view">
      <div className="analysis-section-heading">
        <h3>Analysis Jobs</h3>
        <IconButton
          label="Refresh analysis jobs"
          onClick={() => setRefresh((value) => value + 1)}
        >
          <RefreshCw size={15} />
        </IconButton>
      </div>
      {disabled ? (
        <p className="muted" role="status">
          Analysis jobs are unavailable on this server.
        </p>
      ) : (
        <ErrorNotice error={error} />
      )}
      {!jobs && !error && <Loading label="Loading analysis jobs" />}
      <form
        className="analysis-controls"
        onSubmit={(event) => {
          event.preventDefault();
          void mutate("/jobs", {
            profile,
            level,
            priority,
            context: snapshot.context,
          });
        }}
      >
        <label>
          Profile
          <input aria-label="Job profile" value={profile} readOnly />
        </label>
        <label>
          Analysis
          <select
            aria-label="Job analysis level"
            value={level}
            onChange={(event) => setLevel(event.target.value as JobLevel)}
          >
            <option value="semantic">Semantic</option>
            <option value="syntax">Syntax</option>
          </select>
        </label>
        <label>
          Priority
          <select
            aria-label="Job priority"
            value={priority}
            onChange={(event) => setPriority(event.target.value as JobPriority)}
          >
            <option value="foreground">Foreground</option>
            <option value="workspace">Workspace</option>
            <option value="background">Background</option>
          </select>
        </label>
        <button
          className="text-button"
          type="submit"
          disabled={pending || disabled}
        >
          <Play size={15} />
          Queue analysis
        </button>
      </form>
      <ErrorNotice error={actionError} />
      {jobs?.length === 0 && (
        <p className="muted">No analysis jobs recorded.</p>
      )}
      <div className="job-list">
        {jobs?.map((job) => (
          <article className="job-row" key={job.id}>
            <div className="job-heading">
              <strong>{job.request.profile}</strong>
              <span className={`badge job-${job.status}`}>
                {job.status.replaceAll("_", " ")}
              </span>
              {!["succeeded", "failed", "cancelled"].includes(job.status) && (
                <IconButton
                  label={`Cancel job ${job.id}`}
                  disabled={pending || job.status === "cancelling"}
                  onClick={() =>
                    void mutate(
                      `/jobs/${encodeURIComponent(job.id)}/cancel`,
                      {},
                    )
                  }
                >
                  <Square size={14} />
                </IconButton>
              )}
            </div>
            <div className="job-metadata">
              <span>{job.request.level}</span>
              <span>{job.request.priority}</span>
              <span className="mono">{job.id}</span>
            </div>
            {job.message && <p>{job.message}</p>}
            {job.snapshot_id && (
              <p>
                Published snapshot{" "}
                <a
                  className="mono"
                  href={`?${new URLSearchParams({ snapshot: job.snapshot_id, view: "health" })}`}
                >
                  {job.snapshot_id}
                </a>
              </p>
            )}
            <details>
              <summary>Events ({job.events.length})</summary>
              <ol className="job-events">
                {job.events.map((event) => (
                  <li key={event.sequence}>
                    <strong>{event.stage}</strong>
                    <span>{event.status}</span>
                    <span className="mono">{event.timestamp_ms} ms</span>
                  </li>
                ))}
              </ol>
            </details>
          </article>
        ))}
      </div>
    </section>
  );
}
