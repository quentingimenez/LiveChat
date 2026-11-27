// A range input cannot expose its value to CSS: `accent-color` fills the
// native track, but a custom track has no way to know where the thumb is.
// These helpers write that share as `--range-p`, which the shared slider style
// paints; the value itself stays in the input.

// Share (0..100) of the track before the thumb, clamped. A broken or empty
// scale paints an empty fill instead of a NaN stop that CSS would drop.
export function rangePercent(value, min, max) {
  const low = Number(min);
  const high = Number(max);
  const current = Number(value);
  if (!Number.isFinite(low) || !Number.isFinite(high) || !Number.isFinite(current) || high <= low) {
    return 0;
  }
  const percent = ((current - low) / (high - low)) * 100;
  return Math.min(100, Math.max(0, Math.round(percent * 100) / 100));
}

// Paint one input. A range without explicit bounds uses its browser defaults:
// 0 at the bottom, 100 at the top.
export function paintRange(input) {
  if (!input.style || typeof input.style.setProperty !== 'function') {
    return;
  }
  const min = input.min === '' || input.min === undefined ? 0 : input.min;
  const max = input.max === '' || input.max === undefined ? 100 : input.max;
  input.style.setProperty('--range-p', `${rangePercent(input.value, min, max)}%`);
}

// Paint now, then follow the thumb. `input` events also fire on the double
// click reset, so a reset keeps the fill in step.
export function bindRangeFill(input) {
  const paint = () => paintRange(input);
  paint();
  input.addEventListener('input', paint);
  return paint;
}
