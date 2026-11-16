import type { ComponentChildren } from 'preact';
import { useId } from 'preact/hooks';
import { bytes, initials, percent } from '../format';
import { describeError } from '../errors';
import { useToast } from './Toasts';
import { Icon } from './icons';

export type BadgeTone = 'ok' | 'err' | 'info' | 'admin' | 'air' | 'outline' | 'warn' | 'neutral';

export function Badge({
  tone,
  children,
  title,
}: {
  tone?: BadgeTone;
  children: ComponentChildren;
  title?: string;
}) {
  return (
    <span class={`badge${tone ? ` is-${tone}` : ''}`} title={title}>
      {children}
    </span>
  );
}

export function Dot({ tone }: { tone?: 'ok' | 'err' | 'air' | 'warn' }) {
  return <span class={`dot${tone ? ` is-${tone}` : ''}`} aria-hidden="true" />;
}

export function Meter({
  used,
  max,
  label = 'Occupation',
  className,
}: {
  used: number;
  max: number;
  label?: string;
  className?: string;
}) {
  const value = percent(used, max);
  return (
    <progress
      class={`meter${value >= 85 ? ' is-high' : ''}${className ? ` ${className}` : ''}`}
      max={100}
      value={value}
      aria-label={`${label} : ${value} %`}
    >
      {value} %
    </progress>
  );
}

export function Progress({ value }: { value: number }) {
  const safe = Math.max(0, Math.min(100, Math.round(value)));
  return (
    <progress class="progress" max={100} value={safe} aria-label={`Progression de la lecture : ${safe} %`}>
      {safe} %
    </progress>
  );
}

export function Avatar({ name, className }: { name: string | null | undefined; className?: string }) {
  return (
    <span class={`avatar${className ? ` ${className}` : ''}`} aria-hidden="true">
      {initials(name) || '?'}
    </span>
  );
}

export function Who({ name }: { name: string | null | undefined }) {
  return (
    <span class="who">
      <Avatar name={name} />
      {name || '—'}
    </span>
  );
}

export function GuildIcon({
  name,
  kind,
  className,
}: {
  name: string;
  kind?: string;
  className?: string;
}) {
  return (
    <span
      class={`avatar is-guild${kind === 'none' ? ' is-none' : ''}${className ? ` ${className}` : ''}`}
      aria-hidden="true"
    >
      {initials(name) || '?'}
    </span>
  );
}

export function Table({ children, className }: { children: ComponentChildren; className?: string }) {
  return (
    <div class="table-wrap">
      <table class={`tbl${className ? ` ${className}` : ''}`}>{children}</table>
    </div>
  );
}

export function Card({
  children,
  className,
  id,
}: {
  children: ComponentChildren;
  className?: string;
  id?: string;
}) {
  return (
    <section class={`card${className ? ` ${className}` : ''}`} id={id}>
      {children}
    </section>
  );
}

export function CardHead({
  title,
  meta,
  children,
}: {
  title: ComponentChildren;
  meta?: ComponentChildren;
  children?: ComponentChildren;
}) {
  return (
    <div class="card-head">
      <h2>{title}</h2>
      {meta != null ? <span class="meta">{meta}</span> : null}
      {children}
    </div>
  );
}

export function PageHead({
  title,
  sub,
  actions,
}: {
  title: ComponentChildren;
  sub?: ComponentChildren;
  actions?: ComponentChildren;
}) {
  return (
    <div class="page-head">
      <div>
        <h1>{title}</h1>
        {sub != null ? <p class="sub">{sub}</p> : null}
      </div>
      {actions != null ? <div class="actions">{actions}</div> : null}
    </div>
  );
}

export function Empty({ children }: { children: ComponentChildren }) {
  return <p class="empty">{children}</p>;
}

export function Loading({ label = 'Chargement…' }: { label?: string }) {
  return (
    <p class="loading" role="status">
      {label}
    </p>
  );
}

export function Unavailable({
  detail,
  label = 'Indisponible sur ce serveur',
}: {
  detail?: ComponentChildren;
  label?: string;
}) {
  return (
    <div class="notice is-unavailable" role="status">
      <Icon name="info" />
      <div>
        <strong>{label}</strong>
        <div class="soft">
          {detail ?? 'Cette route du contrat n’est pas encore exposée par ce serveur.'}
        </div>
      </div>
    </div>
  );
}

export function ErrorNotice({
  error,
  onRetry,
  title,
}: {
  error: unknown;
  onRetry?: () => void;
  title?: string;
}) {
  return (
    <div class="notice is-err" role="alert">
      <Icon name="alert" />
      <div>
        <strong>{title ?? 'Chargement impossible'}</strong>
        <div class="soft">{describeError(error)}</div>
      </div>
      {onRetry ? (
        <button type="button" class="btn is-small" onClick={onRetry}>
          Réessayer
        </button>
      ) : null}
    </div>
  );
}

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: ReadonlyArray<SegmentOption<T>>;
  onChange: (value: T) => void;
  label: string;
}) {
  return (
    <div class="seg" role="group" aria-label={label}>
      {options.map((option) => (
        <button
          type="button"
          key={option.value}
          aria-pressed={value === option.value}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function SearchInput({
  value,
  onInput,
  placeholder,
  label,
}: {
  value: string;
  onInput: (value: string) => void;
  placeholder: string;
  label: string;
}) {
  return (
    <div class="search">
      <Icon name="search" />
      <input
        class="field"
        type="search"
        placeholder={placeholder}
        aria-label={label}
        value={value}
        onInput={(event) => onInput(event.currentTarget.value)}
      />
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  hint,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ComponentChildren;
  hint?: ComponentChildren;
}) {
  const id = useId();
  return (
    <div>
      <label class="switch" for={id}>
        <input
          id={id}
          type="checkbox"
          checked={checked}
          onChange={(event) => onChange(event.currentTarget.checked)}
        />
        <span>{label}</span>
      </label>
      {hint != null ? <p class="hint">{hint}</p> : null}
    </div>
  );
}

export function CopyButton({
  text,
  label = 'Copier',
  iconOnly = false,
}: {
  text: string;
  label?: string;
  iconOnly?: boolean;
}) {
  const notify = useToast();
  const copy = () => {
    if (navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
      navigator.clipboard.writeText(text).catch(() => {});
    }
    notify('Copié dans le presse-papiers.');
  };
  return (
    <button
      type="button"
      class={`btn is-small${iconOnly ? ' is-quiet is-icon' : ' is-quiet'}`}
      onClick={copy}
      aria-label={iconOnly ? label : undefined}
      title={label}
    >
      <Icon name="copy" />
      {iconOnly ? null : label}
    </button>
  );
}

export function SizeCell({ value }: { value: number | null | undefined }) {
  return <span class="num soft">{bytes(value)}</span>;
}

export function dateTitle(iso: string | null | undefined): string | undefined {
  return iso ?? undefined;
}
