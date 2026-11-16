import { describe, expect, it } from 'vitest';
import {
  changedCells,
  changedCellKeys,
  cloneRights,
  effectivePermissions,
  enforcement,
  findAction,
  resetRightsToDefaults,
  rightsDirty,
  rightsPayload,
  setRightAllowed,
} from './rights';
import { rightsFixture } from './test-fixtures';

describe('rights matrix', () => {
  it('marks a changed editable cell as dirty', () => {
    const draft = setRightAllowed(rightsFixture, 'overlay', 'viewQueue', 'guest', true);

    expect(rightsDirty(rightsFixture, draft)).toBe(true);
    expect(changedCells(rightsFixture, draft)).toEqual([
      { surface: 'overlay', action: 'viewQueue', role: 'guest', allowed: true },
    ]);
    expect(changedCellKeys(rightsFixture, draft)).toEqual(new Set(['overlay|viewQueue|guest']));
    // The saved copy is untouched.
    expect(findAction(rightsFixture, 'overlay', 'viewQueue')?.cells.guest.allowed).toBe(false);
  });

  it('never changes a fixed cell', () => {
    const draft = setRightAllowed(rightsFixture, 'overlay', 'receiveMedia', 'guest', false);

    expect(findAction(draft, 'overlay', 'receiveMedia')?.cells.guest.allowed).toBe(true);
    expect(rightsDirty(rightsFixture, draft)).toBe(false);
    expect(changedCells(rightsFixture, draft)).toEqual([]);
  });

  it('returns to clean after cancelling or toggling back', () => {
    const changed = setRightAllowed(rightsFixture, 'extension', 'send', 'guest', true);
    expect(rightsDirty(rightsFixture, changed)).toBe(true);

    const cancelled = cloneRights(rightsFixture);
    expect(rightsDirty(rightsFixture, cancelled)).toBe(false);

    const toggledBack = setRightAllowed(changed, 'extension', 'send', 'guest', false);
    expect(rightsDirty(rightsFixture, toggledBack)).toBe(false);
  });

  it('builds the PUT payload, including the Discord admin flag', () => {
    const draft = setRightAllowed(rightsFixture, 'overlay', 'viewQueue', 'guest', true);
    draft.discordAdminIsAdmin = false;

    expect(rightsPayload(rightsFixture, draft)).toEqual({
      cells: [{ surface: 'overlay', action: 'viewQueue', role: 'guest', allowed: true }],
      discordAdminIsAdmin: false,
    });
  });

  it('resets editable cells to their defaults', () => {
    const draft = setRightAllowed(rightsFixture, 'overlay', 'viewQueue', 'member', false);
    const defaults = resetRightsToDefaults(draft);

    expect(findAction(defaults, 'overlay', 'viewQueue')?.cells.member.allowed).toBe(true);
    expect(findAction(defaults, 'overlay', 'receiveMedia')?.cells.guest.allowed).toBe(true);
    expect(rightsDirty(rightsFixture, defaults)).toBe(false);
  });

  it('computes effective permissions for a read-only guest session', () => {
    const memberPermissions = effectivePermissions(rightsFixture, 'member', 'normal', false);
    expect(memberPermissions.find((permission) => permission.action === 'viewQueue')?.granted).toBe(
      true,
    );
    expect(memberPermissions.find((permission) => permission.action === 'viewBoard')?.granted).toBe(
      true,
    );

    const readOnly = effectivePermissions(rightsFixture, 'admin', 'invite_read_only', false);
    expect(readOnly.find((permission) => permission.action === 'viewQueue')?.granted).toBe(false);
    expect(readOnly.find((permission) => permission.action === 'receiveMedia')?.granted).toBe(true);
  });

  it('honours the guild flag for guests viewing the board', () => {
    const withoutFlag = effectivePermissions(rightsFixture, 'guest', 'normal', false);
    expect(withoutFlag.find((permission) => permission.action === 'viewBoard')?.granted).toBe(false);

    const withFlag = effectivePermissions(rightsFixture, 'guest', 'normal', true);
    expect(withFlag.find((permission) => permission.action === 'viewBoard')?.granted).toBe(true);
  });

  it('derives the documented enforcement column', () => {
    expect(enforcement('overlay', 'viewQueue').className).toBe('is-ok');
    expect(enforcement('extension', 'send').label).toBe('À l’appairage seulement');
    expect(enforcement('extension', 'author').label).toBe('Non vérifié aujourd’hui');
  });
});
