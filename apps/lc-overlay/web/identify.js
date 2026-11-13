(() => {
  'use strict';

  const display = window.__livechatIdentify && typeof window.__livechatIdentify === 'object'
    ? window.__livechatIdentify
    : {};
  const index = Number.isInteger(display.index) && display.index >= 0 ? display.index : 0;
  const label = typeof display.label === 'string' && display.label.trim()
    ? display.label.trim()
    : `Écran ${index + 1}`;
  const role = typeof display.role === 'string' ? display.role.trim() : '';

  const numberNode = document.getElementById('identify-number');
  const labelNode = document.getElementById('identify-label');
  const roleNode = document.getElementById('identify-role');

  if (numberNode) {
    numberNode.textContent = String(index + 1);
  }
  if (labelNode) {
    labelNode.textContent = label;
  }
  if (roleNode) {
    roleNode.textContent = role;
    roleNode.hidden = role === '';
  }
})();
