import { useEffect, useRef } from 'react';
import { EditorState } from '@codemirror/state';
import { EditorView, lineNumbers, highlightActiveLine, highlightActiveLineGutter } from '@codemirror/view';
import { defaultHighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { rust } from '@codemirror/lang-rust';
import { FileCode2 } from 'lucide-react';
import { params, request } from '../api/client';
import type { SourceWindow, Span } from '../api/types';
import { byteToCharacter, useResource } from '../state';
import { ErrorNotice, Loading } from './common';

export function SourcePane({ snapshot, context, span, title = 'Source' }: { snapshot: string; context: string; span?: Span; title?: string }) {
  const source = useResource<SourceWindow>(span ? `${snapshot}:${context}:${span.file_id}:${span.start}` : '', signal => request(`/source/${encodeURIComponent(span!.file_id)}${params({ snapshot_id: snapshot, context_id: context, offset: span!.start, lines: 200 })}`, signal));
  const mount = useRef<HTMLDivElement>(null);
  const editor = useRef<EditorView | null>(null);
  useEffect(() => {
    if (!mount.current || !source.data) return;
    const data = source.data;
    const view = new EditorView({
      parent: mount.current,
      state: EditorState.create({
        doc: data.text,
        extensions: [
          EditorState.readOnly.of(true), EditorView.editable.of(false),
          EditorView.contentAttributes.of({ 'aria-label': `${title}: ${data.path}`, 'data-testid': 'source-content' }),
          lineNumbers({ formatNumber: value => String(data.start_line + value - 1) }),
          highlightActiveLine(), highlightActiveLineGutter(), rust(), syntaxHighlighting(defaultHighlightStyle),
          EditorView.theme({ '&': { height: '100%', fontSize: '13px' }, '.cm-scroller': { overflow: 'auto', fontFamily: '"SFMono-Regular", Consolas, "Liberation Mono", monospace', lineHeight: '1.7' }, '.cm-gutters': { backgroundColor: '#f5f7f6', color: '#7b8580', border: 'none', paddingRight: '8px' }, '.cm-content': { padding: '12px 0' }, '.cm-activeLine': { backgroundColor: '#eaf5ef' }, '.cm-selectionBackground': { backgroundColor: '#c1e5d5 !important' }, '&.cm-focused': { outline: 'none' } }),
        ],
      }),
    });
    editor.current = view;
    return () => { view.destroy(); editor.current = null; };
  }, [source.data, title]);
  useEffect(() => {
    if (!editor.current || !source.data || !span) return;
    const anchor = byteToCharacter(source.data.text, span.start - source.data.start_byte);
    const head = byteToCharacter(source.data.text, span.end - source.data.start_byte);
    editor.current.dispatch({ selection: { anchor, head }, effects: EditorView.scrollIntoView(anchor, { y: 'center' }) });
  }, [source.data, span?.start, span?.end]);
  return <section className="source-pane" aria-label={title}>
    <div className="panel-heading"><FileCode2 size={15} /><strong>{source.data?.path ?? title}</strong>{source.data && <span className="muted">Rust · L{source.data.start_line}</span>}</div>
    {source.loading && <Loading label="Loading source" />}
    <ErrorNotice error={source.error} />
    {!span && <div className="empty">No source selection</div>}
    <div className="editor-mount" ref={mount} />
    {source.data?.truncated && <div className="window-notice">Source window: lines {source.data.start_line}–{source.data.start_line + source.data.text.split('\n').length - 1} of {source.data.total_lines}. Surrounding source is not loaded.</div>}
  </section>;
}
