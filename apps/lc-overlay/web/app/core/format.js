// Pure formatting helpers (no DOM, no state).

export function initials(name) {
  const words = String(name || '?').trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? words[0][0] + words[1][0] : (words[0] || '?').slice(0, 2);
  return letters.toUpperCase();
}

export function formatDuration(seconds) {
  if (!seconds || seconds <= 0) {
    return null;
  }
  const total = Math.round(seconds);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
}

export function relativeTime(iso) {
  if (!iso) {
    return 'Jamais vu';
  }
  const minutes = Math.round((Date.now() - new Date(iso).getTime()) / 60000);
  if (minutes < 2) {
    return "À l'instant";
  }
  if (minutes < 60) {
    return `Il y a ${minutes} min`;
  }
  if (minutes < 1440) {
    return `Il y a ${Math.round(minutes / 60)} h`;
  }
  return `Il y a ${Math.round(minutes / 1440)} j`;
}

export function formatCode(code) {
  return code.length === 8 ? `${code.slice(0, 4)}-${code.slice(4)}` : code;
}
