// Pure helper of the board's Ctrl+F shortcut (no DOM, unit-tested in
// dev/tests/find-shortcut.test.mjs).

/**
 * What Ctrl+F (or Cmd+F) does in the board window: `focusSearch` gives the
 * focus to the media search field, `swallow` prevents the key's browser
 * default without moving the focus, and null leaves the key alone.
 *
 * WebView2 opens its « Find on page » bar on Ctrl+F unless the page prevents
 * the default, so the board claims the shortcut whenever it can: a search field
 * shown and no dialog gets the focus; otherwise the key is swallowed so the
 * browser bar never appears in the app window. A key a context menu already
 * handled or a shortcut capture keeps its normal effect.
 */
export function findAction({ key, ctrl, meta, altKey, handled, capturing, dialogOpen, searchAvailable }) {
  if (key !== 'f' && key !== 'F') {
    return null;
  }
  if (!(ctrl || meta) || altKey || handled || capturing) {
    return null;
  }
  return dialogOpen || !searchAvailable ? 'swallow' : 'focusSearch';
}
