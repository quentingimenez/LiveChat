const pinoPretty = require('pino-pretty');

const RESET = '\x1b[0m';
const BOLD = '\x1b[1m';

const PREFIX_COLORS = {
  BOOT:         '\x1b[96m',   // bright cyan
  DB:           '\x1b[35m',   // magenta
  DISCORD:      '\x1b[34m',   // blue
  DISCORD_SYNC: '\x1b[33m',   // yellow
  OVERLAY:      '\x1b[32m',   // green
  PLAYBACK:     '\x1b[95m',   // bright magenta
  SOCKET:       '\x1b[36m',   // cyan
  REST:         '\x1b[94m',   // bright blue
  HTTP:         '\x1b[91m',   // bright red
  SECURITY:     '\x1b[31m',   // red
  MEDIA:        '\x1b[92m',   // bright green
  ADMIN:        '\x1b[93m',   // bright yellow
  PAIRING:      '\x1b[35m',   // magenta
  I18N:         '\x1b[90m',   // gray
  SERVER:       '\x1b[96m',   // bright cyan
};

const PREFIX_RE = /\[([A-Z][A-Z0-9_]*)\]/;

module.exports = (opts) => {
  return pinoPretty({
    ...opts,
    messageFormat: (log, messageKey) => {
      const msg = log[messageKey];
      if (typeof msg !== 'string') return msg;

      return msg.replace(PREFIX_RE, (match, prefix) => {
        const color = PREFIX_COLORS[prefix];
        if (color) return `${BOLD}${color}${match}${RESET}`;
        return match;
      });
    },
  });
};
