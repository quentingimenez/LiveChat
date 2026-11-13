/*
 * LiveChat overlay bridge — façade `window.livechatOverlay` au-dessus de l'IPC
 * Tauri 2, sans modifier le renderer existant.
 * Contrat de référence : docs/rust/overlay-bridge.md.
 *
 * Contraintes :
 *  - script CLASSIQUE (pas de module ES, pas de bundler, aucune dépendance) ;
 *    il doit être évalué AVANT les scripts `type="module"` des pages ;
 *  - aucune référence à Node/Electron : uniquement `window.__TAURI_INTERNALS__`
 *    (Tauri 2) ou `window.__TAURI__` (si `withGlobalTauri` est activé) ;
 *  - si Tauri est absent (navigateur, `file://`), bascule en mode MOCK :
 *    valeurs neutres, aucun réseau, événements simulables pour les tests.
 *
 * Globals de diagnostic (hors API du renderer) — exposés UNIQUEMENT en mode mock
 * ou si `window.__LIVECHAT_BRIDGE_DEBUG = true` est posé avant le chargement
 * (AUD-A6-010 / O-007 ; le build release Tauri n'expose rien) :
 *   window.__livechatOverlayBridge : { version, mode, transport, hasNativeEvents,
 *                                      commands, events, calls, resetCallLog() }
 *   window.__bridgeMode            : 'tauri' | 'mock'
 *   window.__bridgeCalls           : journal des appels (max 200, champs sensibles masqués)
 *   window.__bridgeLastEvent       : dernier événement reçu/émis en mode test
 *   window.__bridgeMockEmit(event, payload) : émet un événement localement (play/stop/settings/placement)
 */
(function () {
  'use strict';

  const LOG_PREFIX = '[lc-overlay-bridge]';
  const VERSION = '0.1.0';

  if (window.livechatOverlay) {
    console.debug(LOG_PREFIX + ' window.livechatOverlay déjà présent — installation ignorée.');
    return;
  }

  // ---------------------------------------------------------------------------
  // Contrat IPC : noms de commandes Tauri et d'événements Tauri.
  // Table de mapping « nom actuel → nom Tauri » : docs/rust/overlay-bridge.md §7.
  // ---------------------------------------------------------------------------
  const COMMANDS = Object.freeze({
    getConfig: 'overlay_get_config',
    rendererReady: 'overlay_renderer_ready',
    reportError: 'overlay_report_error',
    reportPlaybackState: 'overlay_report_playback_state',
    reportPlaybackStop: 'overlay_report_playback_stop',
    consumePairing: 'pairing_consume',
    getMediaBindings: 'media_board_get_bindings',
    setMediaBindings: 'media_board_set_bindings',
    triggerMedia: 'media_board_trigger',
    stopMediaPlayback: 'media_board_stop',
    finishPlacement: 'overlay_placement_finish'
  });

  const EVENTS = Object.freeze({
    play: 'overlay://play',
    stop: 'overlay://stop',
    settings: 'overlay://settings',
    placement: 'overlay://placement'
  });

  const EVENT_LIST = [EVENTS.play, EVENTS.stop, EVENTS.settings, EVENTS.placement];

  // ---------------------------------------------------------------------------
  // Détection Tauri 2.
  //   1. `window.__TAURI_INTERNALS__.invoke` (Tauri 2, avecGlobalTauri indifférent) ;
  //   2. `window.__TAURI__.core.invoke` (Tauri 2, `withGlobalTauri: true`) ;
  //   3. sinon mode MOCK.
  // ---------------------------------------------------------------------------
  const internals = asObject(window.__TAURI_INTERNALS__);
  const globalTauri = asObject(window.__TAURI__);

  function asObject(value) {
    return value && typeof value === 'object' ? value : null;
  }

  function resolveTransport() {
    if (internals && typeof internals.invoke === 'function') {
      return {
        mode: 'tauri',
        kind: 'internals-invoke',
        invoke: function (command, args) {
          return internals.invoke(command, args);
        }
      };
    }

    if (globalTauri && asObject(globalTauri.core) && typeof globalTauri.core.invoke === 'function') {
      return {
        mode: 'tauri',
        kind: 'global-invoke',
        invoke: function (command, args) {
          return globalTauri.core.invoke(command, args);
        }
      };
    }

    return { mode: 'mock', kind: 'none', invoke: null };
  }

  const transport = resolveTransport();
  const invokeFn = transport.invoke;
  const isTauri = transport.mode === 'tauri';
  // Diagnostics actifs uniquement en mode mock (harnais de dev) ou avec un
  // opt-in explicite posé AVANT le chargement de bridge.js :
  //   window.__LIVECHAT_BRIDGE_DEBUG = true
  // Le build release en mode Tauri n'expose donc aucun global de diagnostic
  // (AUD-A6-010 / O-007 : pas de journal d'appels ni de dernier événement).
  const debugEnabled = !isTauri || window.__LIVECHAT_BRIDGE_DEBUG === true;

  // ---------------------------------------------------------------------------
  // Abonnement aux événements Tauri 2.
  //   - voie officielle : window.__TAURI__.event.listen (withGlobalTauri: true) ;
  //   - repli sans API globale : `plugin:event|listen` + transformCallback
  //     (même mécanisme que @tauri-apps/api/event ; permission core:event:default).
  // L'adaptateur normalise le handler : le renderer reçoit le payload brut,
  // exactement comme le `ipcRenderer.on` d'Electron.
  // ---------------------------------------------------------------------------
  function pickEventPayload(eventLike) {
    if (eventLike && typeof eventLike === 'object' && Object.prototype.hasOwnProperty.call(eventLike, 'payload')) {
      return eventLike.payload;
    }

    return eventLike;
  }

  function resolveNativeListen() {
    if (globalTauri && asObject(globalTauri.event) && typeof globalTauri.event.listen === 'function') {
      return function (eventName, onPayload) {
        return globalTauri.event.listen(eventName, function (eventLike) {
          onPayload(pickEventPayload(eventLike));
        });
      };
    }

    if (internals && typeof internals.transformCallback === 'function' && invokeFn) {
      return function (eventName, onPayload) {
        const handlerId = internals.transformCallback(function (eventLike) {
          onPayload(pickEventPayload(eventLike));
        }, false);

        return invokeFn('plugin:event|listen', {
          event: eventName,
          target: { kind: 'Any' },
          handler: handlerId
        }).then(function (eventId) {
          return function () {
            return invokeFn('plugin:event|unlisten', { event: eventName, eventId: eventId });
          };
        });
      };
    }

    return null;
  }

  const nativeListen = isTauri ? resolveNativeListen() : null;

  // ---------------------------------------------------------------------------
  // Journal de diagnostic (mémoire du webview uniquement, champs sensibles masqués).
  // ---------------------------------------------------------------------------
  const MAX_CALLS = 200;
  const callLog = [];
  let callSeq = 0;

  const SENSITIVE_KEY_PATTERN = /(token|code|sig|secret|password)/i;

  function sanitizeForLog(value, depth) {
    const level = typeof depth === 'number' ? depth : 0;

    if (level > 4) {
      return '[…]';
    }

    if (value === null || typeof value === 'undefined') {
      return value === null ? null : '[undefined]';
    }

    if (typeof value === 'string') {
      return value.length > 300 ? value.slice(0, 300) + '…' : value;
    }

    if (typeof value === 'number' || typeof value === 'boolean') {
      return value;
    }

    if (Array.isArray(value)) {
      return value.slice(0, 20).map(function (item) {
        return sanitizeForLog(item, level + 1);
      });
    }

    if (typeof value === 'object') {
      const output = {};

      Object.keys(value).forEach(function (key) {
        output[key] = SENSITIVE_KEY_PATTERN.test(key) ? '***' : sanitizeForLog(value[key], level + 1);
      });

      return output;
    }

    return '[unsupported]';
  }

  function pushCall(method, args) {
    const entry = {
      id: ++callSeq,
      method: method,
      at: new Date().toISOString(),
      args: sanitizeForLog(args, 0),
      status: 'pending',
      result: null,
      error: null
    };

    callLog.push(entry);

    while (callLog.length > MAX_CALLS) {
      callLog.shift();
    }

    return entry;
  }

  function finishCall(entry, status, value) {
    entry.status = status;
    entry.finishedAt = new Date().toISOString();

    if (status === 'ok') {
      entry.result = sanitizeForLog(value, 0);
    } else {
      entry.error = describeError(value);
    }
  }

  function describeError(cause) {
    try {
      return toBridgeError(cause).message;
    } catch (error) {
      return 'Erreur bridge non sérialisable';
    }
  }

  /**
   * Electron rejette un `invoke` avec la valeur d'erreur du handler main
   * (souvent une `Error`). Tauri rejette avec la valeur sérialisée du
   * `Result::Err` (souvent une string). Le renderer existant lit
   * `error?.message` (ex. renderer/pairing.js) : on normalise donc tout en
   * `Error` en conservant le message d'origine dans `.message`.
   */
  function toBridgeError(cause) {
    if (cause instanceof Error) {
      return cause;
    }

    let message = '';

    if (typeof cause === 'string') {
      message = cause;
    } else {
      try {
        message = JSON.stringify(cause);
      } catch (error) {
        message = String(cause);
      }
    }

    const bridgeError = new Error(message || 'Erreur bridge inconnue');
    bridgeError.cause = cause;
    return bridgeError;
  }

  // ---------------------------------------------------------------------------
  // Abonnements locaux (mode mock + outil de test __bridgeMockEmit).
  // ---------------------------------------------------------------------------
  const localListeners = Object.create(null);

  function addLocalListener(eventName, callback) {
    if (!localListeners[eventName]) {
      localListeners[eventName] = [];
    }

    localListeners[eventName].push(callback);

    return function removeLocalListener() {
      const listeners = localListeners[eventName];

      if (!listeners) {
        return;
      }

      const index = listeners.indexOf(callback);

      if (index >= 0) {
        listeners.splice(index, 1);
      }
    };
  }

  function invokeListener(callback, payload, eventName) {
    try {
      callback(payload);
    } catch (error) {
      console.error(LOG_PREFIX + ' listener ' + eventName + ' en erreur : ' + describeError(error));
    }
  }

  function createNativeSubscription(eventName, callback) {
    let disposed = false;
    let unlisten = null;
    let ready = Promise.resolve();

    try {
      ready = Promise.resolve(
        nativeListen(eventName, function (payload) {
          if (disposed) {
            return;
          }

          if (debugEnabled) {
            window.__bridgeLastEvent = {
              event: eventName,
              payload: sanitizeForLog(payload, 0),
              at: new Date().toISOString(),
              source: 'tauri'
            };
          }

          invokeListener(callback, payload, eventName);
        })
      )
        .then(function (dispose) {
          unlisten = typeof dispose === 'function' ? dispose : null;

          if (disposed && unlisten) {
            safeInvoke(unlisten);
          }
        })
        .catch(function (error) {
          console.error(LOG_PREFIX + ' abonnement ' + eventName + ' impossible : ' + describeError(error));
        });
    } catch (error) {
      console.error(LOG_PREFIX + ' abonnement ' + eventName + ' impossible : ' + describeError(error));
    }

    return {
      ready: ready,
      dispose: function () {
        disposed = true;

        if (unlisten) {
          Promise.resolve(safeInvoke(unlisten)).catch(function () {
            /* unlisten déjà consommé ou webview détruit : sans conséquence */
          });
        }
      }
    };
  }

  function subscribeEvent(method, eventName, callback) {
    if (typeof callback !== 'function') {
      console.warn(LOG_PREFIX + ' ' + method + ' : callback invalide (' + typeof callback + ').');
      return function () {};
    }

    const removeLocalListener = addLocalListener(eventName, callback);
    const entry = pushCall(method, { event: eventName });
    let nativeSubscription = null;

    if (isTauri && nativeListen) {
      nativeSubscription = createNativeSubscription(eventName, callback);
    } else if (isTauri) {
      console.warn(
        LOG_PREFIX +
          ' mode Tauri sans API événements (withGlobalTauri: false / transformCallback absent) : ' +
          eventName +
          ' restera silencieux.'
      );
    }

    finishCall(entry, 'ok', { subscribed: true, event: eventName });

    return function unsubscribe() {
      removeLocalListener();

      if (nativeSubscription) {
        nativeSubscription.dispose();
      }

      const disposeEntry = pushCall(method + ':unsubscribe', { event: eventName });
      finishCall(disposeEntry, 'ok', { unsubscribed: true, event: eventName });
    };
  }

  function safeInvoke(fn) {
    try {
      return fn();
    } catch (error) {
      return undefined;
    }
  }

  // ---------------------------------------------------------------------------
  // Appel de commande Tauri (invoke) + simulation en mode mock.
  // ---------------------------------------------------------------------------
  const MOCK_CONFIG = Object.freeze({
    serverUrl: null,
    clientToken: null,
    guildId: null,
    clientId: null,
    guestMode: false,
    showText: true,
    volume: 1,
    mediaBindings: {},
    layout: {
      anchor: 'center',
      maxWidth: 100,
      maxHeight: 100,
      margin: 24
    }
  });

  function cloneJson(value) {
    return JSON.parse(JSON.stringify(value));
  }

  function normalizeMockBindings(bindings) {
    const normalized = {};

    if (!bindings || typeof bindings !== 'object') {
      return normalized;
    }

    Object.keys(bindings).forEach(function (accelerator) {
      const mappedItemId = bindings[accelerator];
      const key = typeof accelerator === 'string' ? accelerator.trim() : '';
      const value = typeof mappedItemId === 'string' ? mappedItemId.trim() : '';

      if (key && value) {
        normalized[key] = value;
      }
    });

    return normalized;
  }

  function normalizeMockErrorPayload(payload) {
    if (!payload || typeof payload !== 'object') {
      return { jobId: 'unknown', code: 'unknown', message: 'mock' };
    }

    return {
      jobId: typeof payload.jobId === 'string' && payload.jobId ? payload.jobId : 'unknown',
      code: typeof payload.code === 'string' && payload.code ? payload.code : 'unknown',
      message: typeof payload.message === 'string' ? payload.message : 'mock'
    };
  }

  function mockCommand(method, commandName, args) {
    const payload = args && typeof args === 'object' ? args.payload : undefined;
    let result;

    switch (commandName) {
      case COMMANDS.getConfig:
        result = cloneJson(MOCK_CONFIG);
        break;

      case COMMANDS.rendererReady:
        result = undefined;
        // Équivalent du main Electron : renderer-ready déclenche l'envoi des settings.
        setTimeout(function () {
          emitMockEvent(EVENTS.settings, cloneJson(MOCK_CONFIG), 'mock:rendererReady');
        }, 0);
        break;

      case COMMANDS.reportError:
        result = undefined;
        console.debug(LOG_PREFIX + ' [mock] reportError', normalizeMockErrorPayload(payload));
        break;

      case COMMANDS.reportPlaybackState:
        result = undefined;
        break;

      case COMMANDS.reportPlaybackStop:
        result = undefined;
        break;

      case COMMANDS.consumePairing:
        result = { ok: true };
        console.info(LOG_PREFIX + ' [mock] consumePairing simulé (aucune requête réseau) :', sanitizeForLog(payload, 0));
        break;

      case COMMANDS.getMediaBindings:
        result = { bindings: {} };
        break;

      case COMMANDS.setMediaBindings:
        result = {
          ok: true,
          bindings: normalizeMockBindings(payload && payload.bindings),
          failedAccelerators: []
        };
        break;

      case COMMANDS.triggerMedia:
        result = { ok: true };
        break;

      case COMMANDS.stopMediaPlayback:
        result = { ok: true };
        break;

      case COMMANDS.finishPlacement:
        result = undefined;
        setTimeout(function () {
          emitMockEvent(EVENTS.placement, { active: false }, 'mock:finishPlacement');
        }, 0);
        break;

      default:
        result = undefined;
        console.warn(LOG_PREFIX + ' [mock] commande inconnue : ' + commandName);
    }

    console.debug(LOG_PREFIX + ' [mock] ' + method + ' → ' + commandName, sanitizeForLog(result, 0));

    return result;
  }

  function callCommand(method, commandName, args, options) {
    const ignoreErrors = Boolean(options && options.ignoreErrors);
    const entry = pushCall(method, args === undefined ? null : args);

    if (!isTauri) {
      let mockResult;

      try {
        mockResult = mockCommand(method, commandName, args);
      } catch (error) {
        finishCall(entry, 'error', error);

        if (ignoreErrors) {
          return Promise.resolve(undefined);
        }

        return Promise.reject(toBridgeError(error));
      }

      finishCall(entry, 'ok', mockResult);
      return Promise.resolve(mockResult);
    }

    return Promise.resolve()
      .then(function () {
        return invokeFn(commandName, args || {});
      })
      .then(
        function (result) {
          finishCall(entry, 'ok', result);
          return result;
        },
        function (error) {
          const bridgeError = toBridgeError(error);
          finishCall(entry, 'error', bridgeError);

          if (ignoreErrors) {
            console.warn(LOG_PREFIX + ' ' + method + ' (' + commandName + ') échoué : ' + bridgeError.message);
            return undefined;
          }

          throw bridgeError;
        }
      );
  }

  /**
   * Les méthodes Electron `rendererReady` / `report*` sont des `send` sans
   * retour : on conserve une API « fire-and-forget » (Promise toujours résolue)
   * pour ne pas provoquer d'unhandled rejection chez le renderer legacy.
   * `getConfig` / `consumePairing` / `media-board:*` propagent leurs erreurs.
   */
  function sendCommand(method, commandName, args) {
    return callCommand(method, commandName, args, { ignoreErrors: true });
  }

  function asPayloadArg(payload) {
    return { payload: payload && typeof payload === 'object' ? payload : {} };
  }

  // ---------------------------------------------------------------------------
  // Événements simulés (tests navigateur) : indépendants du backend, y compris
  // en mode Tauri (les listeners locaux sont toujours enregistrés).
  // ---------------------------------------------------------------------------
  function normalizeEventName(rawEventName) {
    if (typeof rawEventName !== 'string') {
      return null;
    }

    const name = rawEventName.trim();
    const aliases = {
      play: EVENTS.play,
      stop: EVENTS.stop,
      settings: EVENTS.settings,
      'overlay:play': EVENTS.play,
      'overlay:stop': EVENTS.stop,
      'overlay:settings': EVENTS.settings,
      'overlay://play': EVENTS.play,
      'overlay://stop': EVENTS.stop,
      'overlay://settings': EVENTS.settings,
      placement: EVENTS.placement,
      'overlay:placement': EVENTS.placement,
      'overlay://placement': EVENTS.placement
    };

    return aliases[name] || null;
  }

  function emitMockEvent(rawEventName, payload, source) {
    const eventName = normalizeEventName(rawEventName);

    if (!eventName) {
      console.warn(LOG_PREFIX + ' __bridgeMockEmit : événement inconnu « ' + rawEventName + ' » (attendu : play, stop, settings, placement).');
      return 0;
    }

    const listeners = localListeners[eventName] ? localListeners[eventName].slice() : [];
    const entry = pushCall('__bridgeMockEmit ' + eventName, payload === undefined ? null : payload);

    if (debugEnabled) {
      window.__bridgeLastEvent = {
        event: eventName,
        payload: sanitizeForLog(payload, 0),
        at: new Date().toISOString(),
        source: source || 'test'
      };
    }

    listeners.forEach(function (callback) {
      invokeListener(callback, payload, eventName);
    });

    finishCall(entry, 'ok', { notified: listeners.length });

    if (debugEnabled) {
      console.debug(LOG_PREFIX + ' événement simulé ' + eventName + ' → ' + listeners.length + ' listener(s)', sanitizeForLog(payload, 0));
    }

    return listeners.length;
  }

  // ---------------------------------------------------------------------------
  // API publique : les méthodes du preload Electron, plus le placement de zone.
  // Signatures : overlay/src/types/livechat-overlay.d.ts:19-33.
  // ---------------------------------------------------------------------------
  const api = {
    getConfig: function () {
      return callCommand('getConfig', COMMANDS.getConfig, undefined);
    },

    // AUD-A6-004 : fetch authentifié par en-tête Authorization (Bearer) pour
    // les appels REST du board. Le clientToken ne doit plus transiter en query.
    authorizedFetch: function (url, options) {
      const requestOptions = options && typeof options === 'object' ? options : {};
      return api.getConfig().then(function (config) {
        const token =
          config && typeof config.clientToken === 'string' ? config.clientToken.trim() : '';
        if (!token) {
          throw new Error('not_paired');
        }
        const headers = new Headers(requestOptions.headers || undefined);
        headers.set('Authorization', 'Bearer ' + token);
        const requestInit = Object.assign({}, requestOptions, { headers: headers });
        const entry = pushCall('authorizedFetch', sanitizeForLog(url, 0));
        return fetch(url, requestInit)
          .then(function (response) {
            finishCall(entry, response.ok ? 'ok' : 'error', { status: response.status });
            return response;
          })
          .catch(function (error) {
            finishCall(entry, 'error', {
              message: String(error && error.message ? error.message : error)
            });
            throw error;
          });
      });
    },

    rendererReady: function () {
      return sendCommand('rendererReady', COMMANDS.rendererReady, undefined);
    },

    onPlay: function (callback) {
      return subscribeEvent('onPlay', EVENTS.play, callback);
    },

    onStop: function (callback) {
      return subscribeEvent('onStop', EVENTS.stop, callback);
    },

    onSettings: function (callback) {
      return subscribeEvent('onSettings', EVENTS.settings, callback);
    },

    // Zone placement start / end ({ active, layout? }), overlay window only.
    onPlacement: function (callback) {
      return subscribeEvent('onPlacement', EVENTS.placement, callback);
    },

    // Ends the zone placement: { zone: {x, y, width, height} } saves it,
    // { zone: null } cancels.
    finishPlacement: function (payload) {
      return callCommand('finishPlacement', COMMANDS.finishPlacement, asPayloadArg(payload));
    },

    reportError: function (payload) {
      if (!payload || typeof payload !== 'object') {
        console.warn(LOG_PREFIX + ' reportError : payload objet attendu (jobId, code, message).');
      }

      return sendCommand('reportError', COMMANDS.reportError, asPayloadArg(payload));
    },

    reportPlaybackState: function (payload) {
      return sendCommand('reportPlaybackState', COMMANDS.reportPlaybackState, asPayloadArg(payload));
    },

    reportPlaybackStop: function (payload) {
      return sendCommand('reportPlaybackStop', COMMANDS.reportPlaybackStop, asPayloadArg(payload));
    },

    consumePairing: function (payload) {
      return callCommand('consumePairing', COMMANDS.consumePairing, asPayloadArg(payload));
    },

    getMediaBindings: function () {
      return callCommand('getMediaBindings', COMMANDS.getMediaBindings, undefined);
    },

    setMediaBindings: function (payload) {
      return callCommand('setMediaBindings', COMMANDS.setMediaBindings, asPayloadArg(payload));
    },

    triggerMedia: function (payload) {
      return callCommand('triggerMedia', COMMANDS.triggerMedia, asPayloadArg(payload));
    },

    stopMediaPlayback: function () {
      return callCommand('stopMediaPlayback', COMMANDS.stopMediaPlayback, undefined);
    }
  };

  window.livechatOverlay = api;

  // ---------------------------------------------------------------------------
  // Diagnostics (globals distincts de l'API pour ne pas modifier le contrat).
  // Exposés uniquement en mode mock ou avec window.__LIVECHAT_BRIDGE_DEBUG=true
  // (AUD-A6-010 / O-007) : le build release Tauri ne les crée pas.
  // ---------------------------------------------------------------------------
  if (debugEnabled) {
    window.__livechatOverlayBridge = {
      version: VERSION,
      mode: transport.mode,
      transport: transport.kind,
      hasNativeEvents: Boolean(nativeListen),
      commands: COMMANDS,
      events: EVENTS,
      calls: callLog,
      resetCallLog: function () {
        callLog.length = 0;
        callSeq = 0;
      }
    };

    window.__bridgeMode = transport.mode;
    window.__bridgeCalls = callLog;
    window.__bridgeLastEvent = null;
    window.__bridgeMockEmit = function (eventName, payload) {
      return emitMockEvent(eventName, payload, 'test');
    };

    if (isTauri) {
      console.info(LOG_PREFIX + ' v' + VERSION + ' — mode TAURI (' + transport.kind + ', événements natifs : ' + (nativeListen ? 'oui' : 'non') + ').');
    } else {
      console.info(
        LOG_PREFIX +
          ' v' +
          VERSION +
          ' — mode MOCK (Tauri absent) : commandes simulées, événements via window.__bridgeMockEmit("play"|"stop"|"settings", payload).'
      );
    }
  }
})();
