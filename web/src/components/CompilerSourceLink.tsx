import { FileCode2 } from "lucide-react";
import type { CompilerSourceMapping, SourceWindow, Span } from "../api/types";

export function CompilerSourceLink({
  mapping,
  source,
  snapshotId,
  fileId,
  label = "Show compiler source",
  onSpan,
}: {
  mapping: CompilerSourceMapping;
  source?: SourceWindow;
  snapshotId: string;
  fileId: string;
  label?: string;
  onSpan: (span: Span) => void;
}) {
  if (mapping.status === "unavailable")
    return (
      <span className="compiler-source-unavailable muted">
        {mapping.reason}
      </span>
    );
  const location = `${mapping.path}:${mapping.start_byte}..${mapping.end_byte}`;
  if (
    !Number.isSafeInteger(mapping.start_byte) ||
    !Number.isSafeInteger(mapping.end_byte) ||
    mapping.start_byte < 0 ||
    mapping.end_byte < mapping.start_byte
  )
    return (
      <span className="compiler-source-unavailable muted">
        Invalid compiler source range
      </span>
    );
  if (
    source?.snapshot_id !== snapshotId ||
    source.file_id !== fileId ||
    source.path !== mapping.path
  )
    return (
      <span className="compiler-source-unavailable muted">
        <code>{location}</code>
        <br />
        Source file binding unavailable
      </span>
    );
  return (
    <button
      type="button"
      className="analysis-source compiler-source-link"
      aria-label={`${label}: ${location}`}
      title={`${label}: ${location}`}
      onClick={() =>
        onSpan({
          file_id: fileId,
          start: mapping.start_byte,
          end: mapping.end_byte,
        })
      }
    >
      <FileCode2 size={13} />
      <span>{location}</span>
    </button>
  );
}
