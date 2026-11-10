/* Phase 8b — télémétrie des erreurs du renderer vers le bridge.
 *
 * Les pages servent le renderer legacy (script classique `overlay.js` /
 * `board.js` / `pairing.js`). Une erreur non capturée y est sinon invisible
 * hors DevTools ; on la remonte au bridge sous forme de commande
 * `overlay_report_error`, que le shell traduit en trame WebSocket `error`
 * (protocole v2, cf. docs/rust/protocol-v2.md §5).
 *
 * Fichier externe (pas de script inline) pour rester compatible avec la CSP
 * `script-src 'self'`.
 */
(function () {
    'use strict';

    function sendError(code, message) {
        try {
            var bridge = window.livechatOverlay;
            if (!bridge || typeof bridge.reportError !== 'function') {
                return;
            }
            bridge.reportError({
                code: code,
                message: message
            });
        } catch (error) {
            // La télémétrie ne doit jamais casser le renderer.
        }
    }

    function describeError(event) {
        if (!event) {
            return 'unknown error';
        }
        var location = '';
        if (event.filename) {
            location = ' [' + String(event.filename) + ':' + String(event.lineno || 0) + ']';
        }
        return String(event.message || 'unknown error') + location;
    }

    window.addEventListener('error', function (event) {
        sendError('renderer_uncaught_error', describeError(event));
    });

    window.addEventListener('unhandledrejection', function (event) {
        var reason = event && event.reason;
        var message = 'unknown rejection';
        try {
            if (reason && (reason.stack || reason.message || reason)) {
                message = String(reason.stack || reason.message || reason);
            }
        } catch (error) {
            message = 'unserializable rejection';
        }
        sendError('renderer_unhandled_rejection', message);
    });

    // Les scripts legacy attrapent leurs erreurs de boot et les journalisent
    // via `console.error` (ex. « Overlay bootstrap failed: ... »). On les
    // relaie au shell pour qu'elles soient visibles hors DevTools.
    var reportedConsoleErrors = 0;
    var originalConsoleError = console.error.bind(console);

    console.error = function () {
        if (reportedConsoleErrors < 20) {
            reportedConsoleErrors += 1;
            var parts = [];
            for (var index = 0; index < arguments.length; index += 1) {
                try {
                    parts.push(String(arguments[index]));
                } catch (error) {
                    parts.push('[unserializable]');
                }
            }
            sendError('renderer_console_error', parts.join(' ').slice(0, 1000));
        }
        originalConsoleError.apply(null, arguments);
    };
})();
