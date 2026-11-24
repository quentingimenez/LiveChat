// Déplacement « main » au bouton du milieu : le contenu suit le pointeur, le
// cadre défile donc dans le sens opposé au geste. Chromium démarre sinon son
// défilement automatique au clique molette ; `mousedown` est annulé avant tout
// geste, y compris quand il n'y a rien à déplacer.
//
// `canStart` évite d'afficher la main quand la scène tient déjà dans le cadre
// ou que la frise est entière : le clique molette reste neutralisé.

const PAN_CURSOR = 'grabbing';

export function createMiddlePan(element, onPan, { canStart = () => true } = {}) {
  let dragging = false;
  let lastX = 0;
  let lastY = 0;
  let previousCursor = '';

  // Le défilement automatique de Chromium écoute le mousedown : annulé en
  // capture, il ne peut pas démarrer avant nous.
  function preventNative(event) {
    if (event.button === 1) {
      event.preventDefault();
    }
  }

  function finish() {
    if (!dragging) {
      return;
    }
    dragging = false;
    element.classList.remove('is-panning');
    element.style.cursor = previousCursor;
  }

  function onDown(event) {
    if (event.button !== 1) {
      return;
    }
    event.preventDefault();
    if (dragging || !canStart()) {
      return;
    }
    dragging = true;
    lastX = event.clientX;
    lastY = event.clientY;
    previousCursor = element.style.cursor;
    element.style.cursor = PAN_CURSOR;
    element.classList.add('is-panning');
    try {
      element.setPointerCapture(event.pointerId);
    } catch {
      // Le pointeur peut déjà être terminé : le geste s'arrêtera au pointerup.
    }
  }

  function onMove(event) {
    if (!dragging) {
      return;
    }
    // Un pointerup manqué (capture perdue, fenêtre quittée) se rattrape ici :
    // sans bouton du milieu enfoncé, le geste est terminé.
    if (!(event.buttons & 4)) {
      finish();
      return;
    }
    const dx = event.clientX - lastX;
    const dy = event.clientY - lastY;
    if (dx === 0 && dy === 0) {
      return;
    }
    lastX = event.clientX;
    lastY = event.clientY;
    event.preventDefault();
    onPan(dx, dy);
  }

  function onUp() {
    finish();
  }

  element.addEventListener('mousedown', preventNative, { capture: true });
  element.addEventListener('auxclick', preventNative, { capture: true });
  element.addEventListener('pointerdown', onDown, { capture: true });
  element.addEventListener('pointermove', onMove);
  element.addEventListener('pointerup', onUp);
  element.addEventListener('pointercancel', onUp);
  element.addEventListener('lostpointercapture', onUp);

  return {
    isDragging: () => dragging,
    destroy() {
      finish();
      element.removeEventListener('mousedown', preventNative, { capture: true });
      element.removeEventListener('auxclick', preventNative, { capture: true });
      element.removeEventListener('pointerdown', onDown, { capture: true });
      element.removeEventListener('pointermove', onMove);
      element.removeEventListener('pointerup', onUp);
      element.removeEventListener('pointercancel', onUp);
      element.removeEventListener('lostpointercapture', onUp);
    },
  };
}
