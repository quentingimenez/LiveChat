// Properties shared by the admin console and the board specifications.

import { always, eventually, now } from '@antithesishq/bombadil';
import { extract } from '@antithesishq/bombadil/browser';

// Rendering slips that show raw JavaScript values to the user, such as a
// template that interpolates a missing field or a number that went wrong.
const leakedValue = extract((state) => {
  const text = state.document.body?.innerText ?? '';
  const match = text.match(/\bundefined\b|\bNaN\b|\[object Object\]/);
  return match ? match[0] : null;
});

/** No `undefined`, `NaN` or `[object Object]` is ever shown on screen. */
export const noLeakedValues = always(() => leakedValue.current === null);

const blank = extract((state) => (state.document.body?.innerText ?? '').trim() === '');

/** A blank page (crashed render, stuck loading) is back to content within 5 seconds. */
export const noLastingBlankPage = always(
  now(() => blank.current).implies(eventually(() => !blank.current).within(5, 'seconds')),
);
