const test = require('node:test');
const assert = require('node:assert/strict');

const { createDisplayService } = require('../../dist/main/services/display-service.js');

test('display-service getDisplayConfigUpdate builds stable metadata', () => {
  const displays = [
    {
      id: 100,
      label: 'Primary',
      bounds: { x: 0, y: 0, width: 1920, height: 1080 }
    }
  ];

  const service = createDisplayService({
    getAllDisplays: () => displays,
    getPrimaryDisplay: () => displays[0]
  });

  assert.deepEqual(service.getDisplayConfigUpdate(displays[0], 0), {
    displayId: 100,
    displayIndex: 0,
    displayKey: '0,0,1920,1080,0',
    displayLabel: 'Primary'
  });
});

test('display-service getTargetDisplay resolves by id, key, index, then primary', () => {
  const displays = [
    {
      id: 100,
      label: 'Primary',
      bounds: { x: 0, y: 0, width: 1920, height: 1080 }
    },
    {
      id: 200,
      label: 'Secondary',
      bounds: { x: 1920, y: 0, width: 1920, height: 1080 }
    }
  ];

  const service = createDisplayService({
    getAllDisplays: () => displays,
    getPrimaryDisplay: () => displays[0]
  });

  assert.equal(
    service.getTargetDisplay({
      displayId: 200,
      displayKey: null,
      displayIndex: null
    }),
    displays[1]
  );

  const keyForSecondary = service.buildDisplayKey(displays[1], 1);
  assert.equal(
    service.getTargetDisplay({
      displayId: null,
      displayKey: keyForSecondary,
      displayIndex: null
    }),
    displays[1]
  );

  assert.equal(
    service.getTargetDisplay({
      displayId: null,
      displayKey: null,
      displayIndex: 1
    }),
    displays[1]
  );

  assert.equal(
    service.getTargetDisplay({
      displayId: 999,
      displayKey: 'missing',
      displayIndex: 99
    }),
    displays[0]
  );
});
