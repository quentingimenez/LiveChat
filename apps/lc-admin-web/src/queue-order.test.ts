import { describe, expect, it } from 'vitest';

import { moveTo } from './queue-order';

const rows = [{ id: 'a' }, { id: 'b' }, { id: 'c' }, { id: 'd' }];
const ids = (list: { id: string }[]) => list.map((row) => row.id);

describe('moveTo', () => {
  it('place le média déposé à la position de la cible', () => {
    expect(ids(moveTo(rows, 'a', 'c'))).toEqual(['b', 'c', 'a', 'd']);
    expect(ids(moveTo(rows, 'd', 'b'))).toEqual(['a', 'd', 'b', 'c']);
  });

  it('laisse la liste inchangée pour un dépôt sur soi-même ou un inconnu', () => {
    expect(ids(moveTo(rows, 'b', 'b'))).toEqual(['a', 'b', 'c', 'd']);
    expect(ids(moveTo(rows, 'inconnu', 'a'))).toEqual(['a', 'b', 'c', 'd']);
    expect(ids(moveTo(rows, 'a', 'inconnu'))).toEqual(['a', 'b', 'c', 'd']);
  });

  it('ne modifie pas la liste d’entrée', () => {
    moveTo(rows, 'a', 'd');
    expect(ids(rows)).toEqual(['a', 'b', 'c', 'd']);
  });
});
