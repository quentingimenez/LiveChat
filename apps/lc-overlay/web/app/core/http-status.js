// Wording of an HTTP failure that carries no known error code. Pure: also
// loaded by the Node tests.

export function serverStatusMessage(status) {
  if (status === 429) {
    return 'Trop de requêtes vers le serveur (429) : réessaie dans quelques minutes.';
  }
  if (status === 502 || status === 503 || status === 504) {
    return `Serveur injoignable (${status}) : réessaie dans un instant.`;
  }
  return `Le serveur a répondu ${status}.`;
}
