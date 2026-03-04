import { initializePairingController } from './pairing-controller.js';
import { createPairingDom } from './pairing-dom.js';

const dom = createPairingDom(document);
initializePairingController(dom);
