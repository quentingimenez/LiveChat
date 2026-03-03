const COMPOSE_STYLE_ID = 'lce-compose-popover-style';
const DEFAULT_HOLD_DURATION_MS = 520;
const MOVE_CANCEL_THRESHOLD_PX = 12;

interface PopoverCopy {
  title: string;
  placeholder: string;
  submitLabel: string;
  cancelLabel: string;
}

export interface AttachLongPressComposeOptions {
  button: HTMLButtonElement;
  onSubmit: (text: string) => Promise<void>;
  holdDurationMs?: number;
  title?: string;
  placeholder?: string;
  submitLabel?: string;
  cancelLabel?: string;
}

export interface LongPressComposeBinding {
  consumeSuppressedClick: () => boolean;
  closePopover: () => void;
}

let activePopoverClose: (() => void) | null = null;

const ensureComposePopoverStyles = (): void => {
  if (document.getElementById(COMPOSE_STYLE_ID)) {
    return;
  }

  const styleNode = document.createElement('style');
  styleNode.id = COMPOSE_STYLE_ID;
  styleNode.textContent = `
.lce-compose-popover {
  position: fixed;
  z-index: 2147483647;
  width: min(320px, calc(100vw - 20px));
  max-height: min(280px, calc(100vh - 24px));
  border-radius: 12px;
  border: 1px solid rgba(255, 255, 255, 0.18);
  padding: 10px;
  background: rgba(15, 15, 15, 0.96);
  backdrop-filter: blur(10px);
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.5);
  color: #f5f5f5;
  font-family: "Segoe UI", "Helvetica Neue", sans-serif;
  pointer-events: auto;
}
.lce-compose-popover-title {
  margin: 0 0 8px;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.02em;
  color: rgba(255, 255, 255, 0.96);
}
.lce-compose-popover-textarea {
  width: 100%;
  min-height: 78px;
  max-height: 120px;
  resize: vertical;
  border-radius: 8px;
  border: 1px solid rgba(255, 255, 255, 0.22);
  background: rgba(0, 0, 0, 0.28);
  color: #fff;
  font-family: inherit;
  font-size: 12px;
  line-height: 1.4;
  padding: 8px 9px;
  outline: none;
}
.lce-compose-popover-textarea:focus {
  border-color: #66b3ff;
  box-shadow: 0 0 0 2px rgba(102, 179, 255, 0.22);
}
.lce-compose-popover-textarea::placeholder {
  color: rgba(255, 255, 255, 0.56);
}
.lce-compose-popover-error {
  min-height: 16px;
  margin-top: 6px;
  font-size: 11px;
  line-height: 1.3;
  color: #ff8a80;
}
.lce-compose-popover-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 8px;
}
.lce-compose-popover-button {
  border: none;
  border-radius: 999px;
  padding: 6px 12px;
  font-family: inherit;
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
}
.lce-compose-popover-button-cancel {
  background: rgba(255, 255, 255, 0.16);
  color: #fff;
}
.lce-compose-popover-button-submit {
  background: #1d9bf0;
  color: #fff;
}
.lce-compose-popover-button:disabled {
  opacity: 0.65;
  cursor: default;
}
`;
  document.head.appendChild(styleNode);
};

const clamp = (value: number, min: number, max: number): number => {
  return Math.min(max, Math.max(min, value));
};

const toErrorMessage = (error: unknown): string => {
  if (error instanceof Error && error.message.trim()) {
    return error.message.trim();
  }

  if (typeof error === 'string' && error.trim()) {
    return error.trim();
  }

  return 'Erreur pendant l’envoi.';
};

const closeActivePopover = (): void => {
  if (!activePopoverClose) {
    return;
  }

  const close = activePopoverClose;
  activePopoverClose = null;
  close();
};

const buildPopoverCopy = (options: AttachLongPressComposeOptions): PopoverCopy => {
  return {
    title: options.title || 'Envoyer vers LiveChat avec texte',
    placeholder: options.placeholder || 'Ajouter un texte (optionnel)',
    submitLabel: options.submitLabel || 'Envoyer',
    cancelLabel: options.cancelLabel || 'Annuler',
  };
};

const openComposePopover = (options: AttachLongPressComposeOptions): void => {
  ensureComposePopoverStyles();
  closeActivePopover();

  const copy = buildPopoverCopy(options);
  const popover = document.createElement('div');
  popover.className = 'lce-compose-popover';
  popover.setAttribute('role', 'dialog');
  popover.setAttribute('aria-modal', 'false');
  popover.setAttribute('aria-label', copy.title);

  const title = document.createElement('p');
  title.className = 'lce-compose-popover-title';
  title.textContent = copy.title;

  const textarea = document.createElement('textarea');
  textarea.className = 'lce-compose-popover-textarea';
  textarea.placeholder = copy.placeholder;
  textarea.spellcheck = true;

  const errorNode = document.createElement('div');
  errorNode.className = 'lce-compose-popover-error';

  const actions = document.createElement('div');
  actions.className = 'lce-compose-popover-actions';

  const cancelButton = document.createElement('button');
  cancelButton.type = 'button';
  cancelButton.className = 'lce-compose-popover-button lce-compose-popover-button-cancel';
  cancelButton.textContent = copy.cancelLabel;

  const submitButton = document.createElement('button');
  submitButton.type = 'button';
  submitButton.className = 'lce-compose-popover-button lce-compose-popover-button-submit';
  submitButton.textContent = copy.submitLabel;

  actions.append(cancelButton, submitButton);
  popover.append(title, textarea, errorNode, actions);
  document.body.appendChild(popover);

  let closed = false;
  let busy = false;

  const setErrorMessage = (message: string | null): void => {
    errorNode.textContent = message || '';
  };

  const setBusy = (nextBusy: boolean): void => {
    busy = nextBusy;
    textarea.disabled = nextBusy;
    cancelButton.disabled = nextBusy;
    submitButton.disabled = nextBusy;
    submitButton.textContent = nextBusy ? 'Envoi...' : copy.submitLabel;
  };

  const reposition = (): void => {
    if (!options.button.isConnected || !popover.isConnected) {
      close();
      return;
    }

    const rect = options.button.getBoundingClientRect();
    const width = popover.offsetWidth || 320;
    const maxLeft = Math.max(8, window.innerWidth - width - 8);
    const left = clamp(rect.left + rect.width / 2 - width / 2, 8, maxLeft);
    const topAbove = rect.top - popover.offsetHeight - 10;
    const topBelow = rect.bottom + 10;
    const maxTop = Math.max(8, window.innerHeight - popover.offsetHeight - 8);
    const top = topAbove >= 8 ? topAbove : clamp(topBelow, 8, maxTop);

    popover.style.left = `${left}px`;
    popover.style.top = `${top}px`;
  };

  const close = (): void => {
    if (closed) {
      return;
    }

    closed = true;
    if (activePopoverClose === close) {
      activePopoverClose = null;
    }

    window.removeEventListener('resize', reposition);
    window.removeEventListener('scroll', reposition, true);
    document.removeEventListener('pointerdown', handleDocumentPointerDown, true);
    document.removeEventListener('keydown', handleDocumentKeyDown, true);
    popover.remove();
  };

  const handleDocumentPointerDown = (event: PointerEvent): void => {
    const target = event.target as Node | null;

    if (!target) {
      close();
      return;
    }

    if (popover.contains(target) || options.button.contains(target)) {
      return;
    }

    close();
  };

  const handleDocumentKeyDown = (event: KeyboardEvent): void => {
    if (event.key !== 'Escape') {
      return;
    }

    event.preventDefault();
    close();
  };

  const submit = (): void => {
    if (busy) {
      return;
    }

    void (async () => {
      setBusy(true);
      setErrorMessage(null);

      try {
        await options.onSubmit(textarea.value);
        close();
      } catch (error) {
        setErrorMessage(toErrorMessage(error));
      } finally {
        if (!closed) {
          setBusy(false);
        }
      }
    })();
  };

  cancelButton.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();
    close();
  });

  submitButton.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();
    submit();
  });

  textarea.addEventListener('keydown', (event) => {
    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
      event.preventDefault();
      submit();
      return;
    }

    if (event.key !== 'Escape') {
      return;
    }

    event.preventDefault();
    close();
  });

  popover.addEventListener('pointerdown', (event) => {
    event.stopPropagation();
  });

  popover.addEventListener('click', (event) => {
    event.stopPropagation();
  });

  activePopoverClose = close;
  window.addEventListener('resize', reposition);
  window.addEventListener('scroll', reposition, true);
  document.addEventListener('pointerdown', handleDocumentPointerDown, true);
  document.addEventListener('keydown', handleDocumentKeyDown, true);

  reposition();
  window.requestAnimationFrame(() => {
    if (!closed) {
      textarea.focus();
    }
  });
};

export const attachLongPressCompose = (options: AttachLongPressComposeOptions): LongPressComposeBinding => {
  const holdDurationMs = Math.max(250, options.holdDurationMs ?? DEFAULT_HOLD_DURATION_MS);
  let pressTimer: number | null = null;
  let isPointerDown = false;
  let pointerId: number | null = null;
  let startX = 0;
  let startY = 0;
  let suppressClicksUntil = 0;

  const clearPressTimer = (): void => {
    if (pressTimer === null) {
      return;
    }

    window.clearTimeout(pressTimer);
    pressTimer = null;
  };

  const cancelPress = (): void => {
    isPointerDown = false;
    pointerId = null;
    clearPressTimer();
  };

  options.button.addEventListener('pointerdown', (event) => {
    if (event.pointerType === 'mouse' && event.button !== 0) {
      return;
    }

    if (options.button.disabled) {
      return;
    }

    isPointerDown = true;
    pointerId = event.pointerId;
    startX = event.clientX;
    startY = event.clientY;

    clearPressTimer();
    pressTimer = window.setTimeout(() => {
      pressTimer = null;

      if (!isPointerDown || options.button.disabled) {
        return;
      }

      suppressClicksUntil = Date.now() + 1200;
      openComposePopover(options);
    }, holdDurationMs);
  });

  options.button.addEventListener('pointermove', (event) => {
    if (!isPointerDown || pointerId !== event.pointerId || pressTimer === null) {
      return;
    }

    const dx = Math.abs(event.clientX - startX);
    const dy = Math.abs(event.clientY - startY);

    if (dx > MOVE_CANCEL_THRESHOLD_PX || dy > MOVE_CANCEL_THRESHOLD_PX) {
      clearPressTimer();
    }
  });

  options.button.addEventListener('pointerup', (event) => {
    if (pointerId !== event.pointerId) {
      return;
    }

    cancelPress();
  });

  options.button.addEventListener('pointercancel', () => {
    cancelPress();
  });

  options.button.addEventListener('pointerleave', (event) => {
    if (event.pointerType === 'mouse') {
      cancelPress();
    }
  });

  options.button.addEventListener('contextmenu', (event) => {
    if (Date.now() > suppressClicksUntil) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();
  });

  return {
    consumeSuppressedClick: () => {
      if (Date.now() > suppressClicksUntil) {
        return false;
      }

      suppressClicksUntil = 0;
      return true;
    },
    closePopover: () => {
      closeActivePopover();
    },
  };
};
