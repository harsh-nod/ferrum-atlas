import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { AlertCircle, CircleHelp, LoaderCircle } from 'lucide-react';
import type { Coverage } from '../api/types';
import { RequestError } from '../api/client';

export function IconButton({ label, children, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { label: string; children: ReactNode }) {
  return <button type="button" className="icon-button" aria-label={label} title={label} {...props}>{children}</button>;
}

export function ErrorNotice({ error }: { error?: Error }) {
  if (!error) return null;
  return <div className="notice error" role="alert"><AlertCircle size={17} /><div><strong>{error instanceof RequestError ? error.code.replaceAll('_', ' ') : 'Request failed'}</strong><p>{error.message}</p>{error instanceof RequestError && error.correlationId && <small>Reference: {error.correlationId}</small>}</div></div>;
}

export function Loading({ label = 'Loading' }: { label?: string }) {
  return <div className="loading" role="status"><LoaderCircle size={18} className="spinner" />{label}</div>;
}

export function CoverageNotice({ coverage, compact = false }: { coverage?: Coverage; compact?: boolean }) {
  if (!coverage) return null;
  if (compact) return <span className={`badge ${coverage.status}`}><CircleHelp size={12} />{coverage.status}</span>;
  return <section className="coverage-detail" aria-label="Analysis coverage">
    <div className="section-label">Analysis coverage <span className={`badge ${coverage.status}`}>{coverage.status}</span></div>
    {coverage.reasons.length > 0 && <ul className="reason-list">{coverage.reasons.map(item => <li key={item.reason}><span>{item.reason.replaceAll('_', ' ')}</span><b>{item.count}</b></li>)}</ul>}
    {coverage.limitations.map((item, index) => <p className="muted" key={index}>{item}</p>)}
  </section>;
}

export function ResizeHandle({ label, value, minimum, maximum, onChange, horizontal = false }: { label: string; value: number; minimum: number; maximum: number; onChange: (value: number) => void; horizontal?: boolean }) {
  const clamp = (next: number) => Math.min(maximum, Math.max(minimum, next));
  return <div className={`resize-handle ${horizontal ? 'horizontal' : ''}`} role="separator" tabIndex={0} aria-label={label} aria-orientation={horizontal ? 'horizontal' : 'vertical'} aria-valuenow={value} aria-valuemin={minimum} aria-valuemax={maximum}
    onKeyDown={event => {
      if (['ArrowLeft', 'ArrowUp', 'ArrowRight', 'ArrowDown'].includes(event.key)) {
        event.preventDefault();
        onChange(clamp(value + (event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -10 : 10)));
      }
    }}
    onPointerDown={event => {
      const element = event.currentTarget;
      element.setPointerCapture(event.pointerId);
      const origin = horizontal ? event.clientY : event.clientX;
      const move = (next: PointerEvent) => onChange(clamp(value + (horizontal ? next.clientY : next.clientX) - origin));
      const finish = () => { element.removeEventListener('pointermove', move); element.removeEventListener('pointerup', finish); element.removeEventListener('pointercancel', finish); };
      element.addEventListener('pointermove', move);
      element.addEventListener('pointerup', finish);
      element.addEventListener('pointercancel', finish);
    }} />;
}
