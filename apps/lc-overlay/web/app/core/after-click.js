// Runs a callback once the click that caused a blur has been delivered.
//
// A text field that rebuilds the page from its own `blur` handler removes the
// control under the pointer between `mousedown` and `mouseup`: the click then
// lands on the rebuilt page instead of the button. Deferring the rebuild until
// the click has been dispatched keeps the pressed control in place. The timer
// covers a blur without a click (Tab, another window).

export function afterNextClick(target, callback, options = {}) {
  const idleMs = options.idleMs ?? 1200;
  const ignore = typeof options.ignore === 'function' ? options.ignore : null;
  let done = false;
  const finish = () => {
    if (done) {
      return false;
    }
    done = true;
    target.removeEventListener('click', onClick);
    clearTimeout(timer);
    callback();
    return true;
  };
  const onClick = (event) => {
    if (ignore && ignore(event)) {
      return;
    }
    finish();
  };
  target.addEventListener('click', onClick);
  const timer = setTimeout(finish, idleMs);
  return finish;
}
