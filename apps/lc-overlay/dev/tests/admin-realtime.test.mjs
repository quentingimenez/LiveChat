//! Protège la chaîne « admin.changed » : mutation serveur → room WS → shell →
//! board, avec un rechargement ciblé de l'onglet Administration.
//!
//! Le harnais n'a pas de DOM ni de WebSocket : les assertions portent sur les
//! modules purs et sur les contrats de source (même style que guild-switch).

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const path = (relative) => new URL(`../../../../${relative}`, import.meta.url);

function source(relative) {
  return readFileSync(path(relative), 'utf8');
}

describe('admin.changed — protocole', () => {
  it('porte le nom de fil et les cinq kinds attendus', () => {
    const protocol = source('crates/lc-protocol/src/lib.rs');
    assert.match(protocol, /AdminChanged\(AdminChangedPayload\)/);
    assert.match(protocol, /ADMIN_CHANGED: &str = "admin\.changed"/);
    assert.match(protocol, /enum AdminChangeKind/);
    for (const kind of ['Members', 'Invites', 'PairingCodes', 'Bans', 'Audit']) {
      assert.match(protocol, new RegExp(`^\\s*${kind},`, 'm'));
    }
  });

  it('est diffusé dans la room de la guilde', () => {
    const hub = source('apps/lc-server/src/ws/hub.rs');
    assert.match(hub, /ServerMessage::AdminChanged\(_\) => "admin"/);
    assert.match(hub, /admin_changed_is_sent_to_the_guild_room/);
  });
});

describe('admin.changed — mutations serveur', () => {
  it('émet après chaque famille de mutation', () => {
    const admin = source('apps/lc-server/src/http/overlay/admin.rs');
    assert.match(admin, /fn broadcast_admin_changed\(state: &AppState/);
    for (const kind of ['Members', 'Bans', 'Invites', 'PairingCodes', 'Audit']) {
      assert.match(admin, new RegExp(`broadcast_admin_changed\\([^\\n]*AdminChangeKind::${kind}`));
    }
  });
});

describe('admin.changed — shell et board', () => {
  it('relaie l’événement vers la board', () => {
    const events = source('apps/lc-overlay/src/events.rs');
    assert.match(events, /EVENT_ADMIN_CHANGED/);
    assert.match(events, /"overlay:\/\/admin-changed"/);
    assert.match(events, /BOARD_LABEL/);

    const ws = source('apps/lc-overlay/src/net/ws.rs');
    assert.match(ws, /ServerMessage::AdminChanged\(payload\)/);
    assert.match(ws, /on_admin_changed/);
  });

  it('ne recharge que l’onglet Administration visible et déduplique par force', () => {
    const main = source('apps/lc-overlay/web/app/main.js');
    const listener = main.slice(main.indexOf("listen('overlay://admin-changed'"));
    assert.match(listener, /payload\.guildId !== state\.activeGuildId/);
    assert.match(listener, /state\.tab !== 'admin'/);
    assert.match(listener, /setTimeout\(/, 'le debounce manque');
    assert.match(listener, /300/, 'le debounce doit laisser passer une rafale');
    assert.match(listener, /loadAdmin\(\{ quiet: true, force: true \}\)/);

    const data = source('apps/lc-overlay/web/app/data.js');
    assert.match(data, /adminForcedSeq/);
    assert.match(data, /adminReadToken/);
    assert.match(data, /admin:\$\{guildId\}:\$\{adminTab\}:fresh:\$\{\+\+adminForcedSeq\}/);
    assert.match(data, /token !== adminReadToken/);
  });
});
