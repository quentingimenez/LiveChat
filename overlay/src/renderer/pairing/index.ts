import { initializePairingController } from './pairing-controller';
import { createPairingDom } from './pairing-dom';

const dom = createPairingDom(document);
initializePairingController(dom);
