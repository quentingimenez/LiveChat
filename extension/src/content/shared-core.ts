// Noyau partagé des scripts de contenu. Chaque bloc déplacé ci-dessous était
// byte-identique dans twitter.ts / youtube.ts / tiktok.ts sur 0296afa ; il a
// été déplacé tel quel, sans changement de comportement.
//
// Restent volontairement par site (divergences réelles documentées dans les
// fichiers concernés) : readRuntime / hasExtensionAuth / registerToastListener
// / sendQuick / sendCompose (TikTok gère l'invalidation de contexte extension),
// ainsi que ensureStyles / createActionButton / les scanners (DOM spécifique).
import { onTrustedPageEvent } from './shared';

interface LongPressComposeOptions {
  button: HTMLButtonElement;
  onSubmit: (text: string) => Promise<void>;
  holdDurationMs?: number;
  title?: string;
  placeholder?: string;
  submitLabel?: string;
  cancelLabel?: string;
}

interface LongPressComposeBinding {
  consumeSuppressedClick: () => boolean;
  closePopover: () => void;
}

// Popover de composition au long-press : bloc byte-identique dans les trois sites.
const attachLongPressCompose = (() => {
  const composeStyleId = 'lce-compose-popover-style';
  const defaultHoldDurationMs = 520;
  const moveCancelThresholdPx = 12;
  let activePopoverClose: (() => void) | null = null;

  const ensureComposePopoverStyles = (): void => {
    if (document.getElementById(composeStyleId)) {
      return;
    }

    const styleNode = document.createElement('style');
    styleNode.id = composeStyleId;
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

  const composeClamp = (value: number, min: number, max: number): number => {
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

  const openComposePopover = (options: LongPressComposeOptions): void => {
    ensureComposePopoverStyles();
    closeActivePopover();

    const popover = document.createElement('div');
    popover.className = 'lce-compose-popover';
    popover.setAttribute('role', 'dialog');
    popover.setAttribute('aria-modal', 'false');
    popover.setAttribute('aria-label', options.title || 'Envoyer vers LiveChat avec texte');

    const title = document.createElement('p');
    title.className = 'lce-compose-popover-title';
    title.textContent = options.title || 'Envoyer vers LiveChat avec texte';

    const textarea = document.createElement('textarea');
    textarea.className = 'lce-compose-popover-textarea';
    textarea.placeholder = options.placeholder || 'Ajouter un texte (optionnel)';
    textarea.spellcheck = true;

    const errorNode = document.createElement('div');
    errorNode.className = 'lce-compose-popover-error';

    const actions = document.createElement('div');
    actions.className = 'lce-compose-popover-actions';

    const cancelButton = document.createElement('button');
    cancelButton.type = 'button';
    cancelButton.className = 'lce-compose-popover-button lce-compose-popover-button-cancel';
    cancelButton.textContent = options.cancelLabel || 'Annuler';

    const submitButton = document.createElement('button');
    submitButton.type = 'button';
    submitButton.className = 'lce-compose-popover-button lce-compose-popover-button-submit';
    submitButton.textContent = options.submitLabel || 'Envoyer';

    actions.append(cancelButton, submitButton);
    popover.append(title, textarea, errorNode, actions);
    document.body.appendChild(popover);

    let closed = false;
    let busy = false;

    const setBusy = (nextBusy: boolean): void => {
      busy = nextBusy;
      textarea.disabled = nextBusy;
      cancelButton.disabled = nextBusy;
      submitButton.disabled = nextBusy;
      submitButton.textContent = nextBusy ? 'Envoi...' : options.submitLabel || 'Envoyer';
    };

    const reposition = (): void => {
      if (!options.button.isConnected || !popover.isConnected) {
        close();
        return;
      }

      const rect = options.button.getBoundingClientRect();
      const width = popover.offsetWidth || 320;
      const maxLeft = Math.max(8, window.innerWidth - width - 8);
      const left = composeClamp(rect.left + rect.width / 2 - width / 2, 8, maxLeft);
      const topAbove = rect.top - popover.offsetHeight - 10;
      const topBelow = rect.bottom + 10;
      const maxTop = Math.max(8, window.innerHeight - popover.offsetHeight - 8);
      const top = topAbove >= 8 ? topAbove : composeClamp(topBelow, 8, maxTop);

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

      if (!target || (popover.contains(target) || options.button.contains(target))) {
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
        errorNode.textContent = '';

        try {
          await options.onSubmit(textarea.value);
          close();
        } catch (error) {
          errorNode.textContent = toErrorMessage(error);
        } finally {
          if (!closed) {
            setBusy(false);
          }
        }
      })();
    };

    onTrustedPageEvent<MouseEvent>(cancelButton, 'click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      close();
    });

    onTrustedPageEvent<MouseEvent>(submitButton, 'click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      submit();
    });

    onTrustedPageEvent<KeyboardEvent>(textarea, 'keydown', (event) => {
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

  return (options: LongPressComposeOptions): LongPressComposeBinding => {
    const holdDurationMs = Math.max(250, options.holdDurationMs ?? defaultHoldDurationMs);
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

    onTrustedPageEvent<PointerEvent>(options.button, 'pointerdown', (event) => {
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

      if (dx > moveCancelThresholdPx || dy > moveCancelThresholdPx) {
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
})();

export const buttonResetTimers = new WeakMap<HTMLButtonElement, number>();
export const buttonClickTimers = new WeakMap<HTMLButtonElement, number>();
export const SINGLE_CLICK_DELAY_MS = 260;

type ToastLevel = 'success' | 'error' | 'info';

const TOAST_STYLE_ID = 'lce-toast-shared-style';
const TOAST_CONTAINER_ID = 'lce-toast-container';

let toastHideTimeout: number | null = null;

export const ensureToastStyles = (): void => {
  if (document.getElementById(TOAST_STYLE_ID)) {
    return;
  }

  const styleNode = document.createElement('style');
  styleNode.id = TOAST_STYLE_ID;
  styleNode.textContent = `
.lce-toast-container {
  position: fixed;
  right: 16px;
  bottom: 16px;
  z-index: 2147483647;
  display: flex;
  flex-direction: column;
  gap: 8px;
  pointer-events: none;
}
.lce-toast {
  min-width: 200px;
  max-width: 360px;
  border-radius: 10px;
  padding: 10px 12px;
  font-family: "Segoe UI", "Helvetica Neue", sans-serif;
  font-size: 13px;
  font-weight: 600;
  color: #fff;
  box-shadow: 0 12px 24px rgba(0, 0, 0, 0.3);
}
.lce-toast-success { background: linear-gradient(135deg, #2e7d32, #43a047); }
.lce-toast-error { background: linear-gradient(135deg, #b71c1c, #e53935); }
.lce-toast-info { background: linear-gradient(135deg, #1565c0, #1e88e5); }
`;
  document.head.appendChild(styleNode);
};

export const getToastContainer = (): HTMLDivElement => {
  let container = document.getElementById(TOAST_CONTAINER_ID) as HTMLDivElement | null;

  if (!container) {
    container = document.createElement('div');
    container.id = TOAST_CONTAINER_ID;
    container.className = 'lce-toast-container';
    document.body.appendChild(container);
  }

  return container;
};

export const showToast = (level: ToastLevel, message: string): void => {
  if (!message || !message.trim()) {
    return;
  }

  ensureToastStyles();

  const container = getToastContainer();
  const toastNode = document.createElement('div');
  toastNode.className = `lce-toast lce-toast-${level}`;
  toastNode.textContent = message;
  container.replaceChildren(toastNode);

  if (toastHideTimeout !== null) {
    window.clearTimeout(toastHideTimeout);
  }

  toastHideTimeout = window.setTimeout(() => {
    toastNode.remove();
  }, 3500);
};

export const isShowToastPayload = (value: unknown): value is { type: string; level?: unknown; message?: unknown } => {
  if (!value || typeof value !== 'object') {
    return false;
  }

  return (value as { type?: unknown }).type === 'lce/show-toast';
};

export const isAuthStateResponse = (value: unknown): value is { ok: boolean; hasSettings: boolean } => {
  if (!value || typeof value !== 'object') {
    return false;
  }

  const payload = value as Record<string, unknown>;
  return typeof payload.ok === 'boolean' && typeof payload.hasSettings === 'boolean';
};

export const clearButtonClickTimer = (button: HTMLButtonElement): void => {
  const activeTimer = buttonClickTimers.get(button);

  if (typeof activeTimer === 'number') {
    window.clearTimeout(activeTimer);
    buttonClickTimers.delete(button);
  }
};

export const scheduleSingleClickAction = (button: HTMLButtonElement, action: () => void): void => {
  clearButtonClickTimer(button);
  const timer = window.setTimeout(() => {
    buttonClickTimers.delete(button);
    action();
  }, SINGLE_CLICK_DELAY_MS);

  buttonClickTimers.set(button, timer);
};

export const clearButtonResetTimer = (button: HTMLButtonElement): void => {
  const activeTimer = buttonResetTimers.get(button);

  if (typeof activeTimer === 'number') {
    window.clearTimeout(activeTimer);
    buttonResetTimers.delete(button);
  }
};

export { attachLongPressCompose };
export type { LongPressComposeOptions, LongPressComposeBinding, ToastLevel };
