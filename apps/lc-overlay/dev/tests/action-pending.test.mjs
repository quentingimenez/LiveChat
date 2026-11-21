// Ce que ce fichier protège : une action en vol est visible sur son contrôle,
// un second clic ne repart pas, l’état optimiste est annulé en cas d’échec et
// le contrôle retrouve son état initial dans tous les cas.

import assert from 'node:assert/strict';
import test from 'node:test';

import { actionErrorMessage, isActionPending, perform } from '../../web/app/core/action.js';

function fakeControl() {
  return {
    disabled: false,
    isConnected: true,
    attributes: new Map(),
    classes: new Set(),
    classList: {
      add(name) {
        this.owner.classes.add(name);
      },
      remove(name) {
        this.owner.classes.delete(name);
      },
    },
    setAttribute(name, value) {
      this.attributes.set(name, value);
    },
    removeAttribute(name) {
      this.attributes.delete(name);
    },
  };
}

test('le contrôle est occupé pendant l’action puis restauré', async () => {
  const control = fakeControl();
  control.classList.owner = control;
  let busyDuringWork = false;
  const result = await perform(control, 'play:a', async () => {
    busyDuringWork = control.disabled && control.attributes.get('aria-busy') === 'true';
    return 'ok';
  });
  assert.equal(result, 'ok');
  assert.equal(busyDuringWork, true);
  assert.equal(control.disabled, false);
  assert.equal(control.attributes.has('aria-busy'), false);
  assert.equal(control.attributes.has('data-pending'), false);
  assert.equal(isActionPending('play:a'), false);
});

test('un second clic sur la même action ne repart pas', async () => {
  const control = fakeControl();
  control.classList.owner = control;
  let release;
  let calls = 0;
  const work = () => {
    calls += 1;
    return new Promise((resolve) => {
      release = resolve;
    });
  };
  const first = perform(control, 'play:a', work);
  const second = await perform(control, 'play:a', work);
  assert.equal(second, undefined);
  assert.equal(calls, 1);
  release('ok');
  assert.equal(await first, 'ok');
});

test('un contrôle déjà désactivé le reste après l’action', async () => {
  const control = fakeControl();
  control.classList.owner = control;
  control.disabled = true;
  await perform(control, 'delete:a', async () => 'ok');
  assert.equal(control.disabled, true);
});

test('l’optimiste précède la requête et le rollback la défait en cas d’échec', async () => {
  const control = fakeControl();
  control.classList.owner = control;
  const order = [];
  let rolledBack = false;
  const result = await perform(control, 'favorite:a', async () => {
    order.push('work');
    throw new Error('serveur');
  }, {
    optimistic: () => {
      order.push('optimistic');
    },
    rollback: () => {
      rolledBack = true;
      order.push('rollback');
    },
    onError: (error) => {
      assert.equal(error.message, 'serveur');
      order.push('error');
    },
  });
  assert.equal(result, undefined);
  assert.deepEqual(order, ['optimistic', 'work', 'rollback', 'error']);
  assert.equal(rolledBack, true);
  assert.equal(control.disabled, false);
});

test('une action lente est marquée après le délai', async () => {
  const control = fakeControl();
  control.classList.owner = control;
  let release;
  const promise = perform(control, 'play:a', () => new Promise((resolve) => {
    release = resolve;
  }), { slowAfter: 5 });
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(control.classes.has('is-slow'), true);
  release('ok');
  await promise;
});

test('onFinally court même après un échec', async () => {
  const control = fakeControl();
  control.classList.owner = control;
  let cleaned = false;
  await perform(control, 'favorite:a', async () => {
    throw new Error('nope');
  }, {
    onFinally: () => {
      cleaned = true;
    },
  });
  assert.equal(cleaned, true);
});

test('actionErrorMessage préfère le message de l’erreur', () => {
  assert.equal(actionErrorMessage(new Error('oups')), 'oups');
  assert.equal(actionErrorMessage('texte'), 'texte');
  assert.equal(actionErrorMessage(null), 'Action impossible.');
});
