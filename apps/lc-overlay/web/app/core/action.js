// In-flight state for user actions.
//
// A click used to await the server with no visual change until the toast. This
// helper marks the control busy for the whole request, keeps a duplicate click
// from firing twice, and lets the view roll an optimistic change back.

const pending = new Map();
const SLOW_AFTER_MS = 5_000;

export function isActionPending(key) {
  return pending.has(key);
}

export function pendingActionKeys() {
  return Array.from(pending.keys());
}

// `control` may be null for keyboard shortcuts. `optimistic` runs before the
// request, `rollback` when it fails, `onSuccess` after it lands. The function
// never throws: callers inspect the returned value (undefined on failure).
export async function perform(control, key, work, options = {}) {
  if (pending.has(key)) {
    return undefined;
  }
  const record = {
    key,
    control,
    timer: null,
  };
  pending.set(key, record);
  const hadDisabled = Boolean(control && control.disabled);
  if (control) {
    control.disabled = true;
    setAttribute(control, 'aria-busy', 'true');
    setAttribute(control, 'data-pending', 'true');
  }
  record.timer = setTimeout(() => {
    if (control) {
      addClass(control, 'is-slow');
    }
  }, options.slowAfter || SLOW_AFTER_MS);

  try {
    if (options.optimistic) {
      await options.optimistic();
    }
    const result = await work();
    if (options.onSuccess) {
      await options.onSuccess(result);
    }
    return result;
  } catch (error) {
    if (options.rollback) {
      try {
        await options.rollback(error);
      } catch {
        // A failed rollback must not hide the original error.
      }
    }
    if (options.onError) {
      await options.onError(error);
    }
    return undefined;
  } finally {
    clearTimeout(record.timer);
    pending.delete(key);
    if (control) {
      control.disabled = hadDisabled;
      removeAttribute(control, 'aria-busy');
      removeAttribute(control, 'data-pending');
      removeClass(control, 'is-slow');
    }
    if (options.onFinally) {
      try {
        await options.onFinally();
      } catch {
        // Cleanup is best effort.
      }
    }
  }
}

export function actionErrorMessage(error) {
  if (typeof error === 'string') {
    return error;
  }
  return (error && error.message) || 'Action impossible.';
}

function setAttribute(control, name, value) {
  if (typeof control.setAttribute === 'function') {
    control.setAttribute(name, value);
  }
}

function removeAttribute(control, name) {
  if (typeof control.removeAttribute === 'function') {
    control.removeAttribute(name);
  }
}

function addClass(control, name) {
  if (control.classList) {
    control.classList.add(name);
  }
}

function removeClass(control, name) {
  if (control.classList) {
    control.classList.remove(name);
  }
}
