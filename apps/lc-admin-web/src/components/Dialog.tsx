import type { ComponentChildren } from 'preact';
import { useEffect, useId, useRef, useState } from 'preact/hooks';
import { Icon } from './icons';
import { CopyButton } from './ui';

export function Dialog({
  open,
  title,
  subtitle,
  onClose,
  wide,
  footer,
  children,
}: {
  open: boolean;
  title: ComponentChildren;
  subtitle?: ComponentChildren;
  onClose: () => void;
  wide?: boolean;
  footer?: ComponentChildren;
  children: ComponentChildren;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    if (!open) {
      return;
    }
    const element = ref.current;
    if (!element) {
      return;
    }
    if (typeof element.showModal === 'function' && !element.open) {
      try {
        element.showModal();
      } catch {
        element.setAttribute('open', '');
      }
    } else if (!element.open) {
      element.setAttribute('open', '');
    }
    return () => {
      if (element.open) {
        if (typeof element.close === 'function') {
          element.close();
        } else {
          element.removeAttribute('open');
        }
      }
    };
  }, [open]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const element = ref.current;
    if (!element) {
      return;
    }
    const onCancel = (event: Event) => {
      event.preventDefault();
      onClose();
    };
    element.addEventListener('cancel', onCancel);
    return () => element.removeEventListener('cancel', onCancel);
  }, [open, onClose]);

  if (!open) {
    return null;
  }

  return (
    <dialog
      ref={ref}
      class={wide ? 'is-wide' : undefined}
      aria-labelledby={titleId}
      onClick={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div class="dlg-head">
        <div>
          <h2 id={titleId}>{title}</h2>
          {subtitle != null ? <p>{subtitle}</p> : null}
        </div>
        <button type="button" class="btn is-quiet is-icon" onClick={onClose} aria-label="Fermer">
          <Icon name="x" />
        </button>
      </div>
      <div class="dlg-body">{children}</div>
      {footer != null ? <div class="dlg-foot">{footer}</div> : null}
    </dialog>
  );
}

export function SecretDialog({
  open,
  title,
  subtitle,
  value,
  hint,
  onClose,
  label = 'Copier',
}: {
  open: boolean;
  title: string;
  subtitle?: ComponentChildren;
  value: string;
  hint?: ComponentChildren;
  onClose: () => void;
  label?: string;
}) {
  return (
    <Dialog
      open={open}
      title={title}
      subtitle={subtitle}
      onClose={onClose}
      footer={
        <button type="button" class="btn is-primary" onClick={onClose}>
          Terminé
        </button>
      }
    >
      <div class="secret">
        <span>{value}</span>
        <CopyButton text={value} label={label} />
      </div>
      {hint != null ? <p class="hint">{hint}</p> : null}
    </Dialog>
  );
}

export function ConfirmDialog({
  open,
  title,
  body,
  confirmLabel,
  danger,
  expect,
  extra,
  busy,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: ComponentChildren;
  body: ComponentChildren;
  confirmLabel: string;
  danger?: boolean;
  expect?: string;
  extra?: ComponentChildren;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const [typed, setTyped] = useState('');
  const inputId = useId();

  useEffect(() => {
    if (open) {
      setTyped('');
    }
  }, [open, expect]);

  const valid = !expect || typed.trim() === expect;

  return (
    <Dialog
      open={open}
      title={title}
      onClose={onCancel}
      footer={
        <>
          <button type="button" class="btn" onClick={onCancel} disabled={busy}>
            Annuler
          </button>
          <button
            type="button"
            class={`btn ${danger ? 'is-danger-solid' : 'is-primary'}`}
            disabled={!valid || busy}
            onClick={onConfirm}
          >
            {busy ? 'En cours…' : confirmLabel}
          </button>
        </>
      }
    >
      <p class="soft">{body}</p>
      {extra}
      {expect ? (
        <div class="dlg-section">
          <label for={inputId}>
            Pour confirmer, saisis l’identifiant <span class="mono">{expect}</span>
          </label>
          <input
            class="field is-after-label mono"
            id={inputId}
            autocomplete="off"
            spellcheck={false}
            value={typed}
            onInput={(event) => setTyped(event.currentTarget.value)}
          />
        </div>
      ) : null}
    </Dialog>
  );
}
