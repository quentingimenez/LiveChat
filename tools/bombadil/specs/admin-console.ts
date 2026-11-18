// Bombadil specification of the admin console (apps/lc-admin-web), explored
// against the real lc-server binary with a logged-in session (see
// ../run-admin.mjs, which seeds a few guilds first).

import { weighted } from '@antithesishq/bombadil';
import { isAction } from '@antithesishq/bombadil/actions';
import {
  back,
  clicks,
  forward,
  inputs,
  reload,
  scroll,
  waitOnce,
} from '@antithesishq/bombadil/browser/defaults/actions';

export {
  noConsoleErrors,
  noHttpErrorCodes,
  noUncaughtExceptions,
  noUnhandledPromiseRejections,
} from '@antithesishq/bombadil/browser/defaults/properties';
export { noLastingBlankPage, noLeakedValues } from './common.ts';

// Logging out ends the run: the login screen probes the session and gets the
// expected 401, which `noHttpErrorCodes` would report. The session cookie is
// set by the runner, so the explorer never has to log in.
const staysLoggedIn = clicks.filter(
  (action) =>
    !(isAction(action, 'Click') && action.Click.fingerprint.accessibleName === 'Se déconnecter'),
);

export const adminActions = weighted([
  [10, staysLoggedIn],
  [4, inputs],
  [2, scroll],
  [1, back],
  [1, forward],
  [1, reload],
  [1, waitOnce],
]);
