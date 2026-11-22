import { fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { QueueJob } from '../../api/types';
import { guildDetailFixture, guildFixture } from '../../test-fixtures';
import { QueueTab } from './QueueTab';

// La file est réordonnée par glisser-déposer : la ligne déposée prend la place
// de la cible et c'est la file entière, dans l'ordre affiché, qui part au serveur.
const mocks = vi.hoisted(() => ({
  reorderQueue: vi.fn(async () => ({ reordered: true as const, count: 3 })),
}));

vi.mock('../../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../api/client')>();
  return {
    ...actual,
    clearQueue: vi.fn(async () => ({ cleared: 0 })),
    removeQueueJob: vi.fn(async () => ({ removed: true })),
    reorderQueue: mocks.reorderQueue,
    stopGuild: vi.fn(async () => ({ stopped: true })),
  };
});

function job(id: string, title: string): QueueJob {
  return {
    id,
    status: 'pending',
    title,
    sourceTitle: null,
    text: null,
    authorName: 'Max',
    authorImage: null,
    durationSec: 5,
    mediaAssetId: `asset-${id}`,
    origin: 'board',
    kind: 'video',
    media: null,
    submittedAt: '2026-11-15T11:59:30.000Z',
    startedAt: null,
    finishedAt: null,
  };
}

function renderQueue(refresh: () => Promise<void> = async () => {}) {
  const detail = {
    ...guildDetailFixture,
    queue: {
      playing: null,
      pending: [job('a', 'Alpha'), job('b', 'Bravo'), job('c', 'Charlie')],
      serverTime: '2026-11-15T12:00:00.000Z',
    },
  };
  render(
    <QueueTab
      guild={guildFixture}
      detail={detail}
      detailUnavailable={false}
      refresh={refresh}
      notify={() => {}}
    />,
  );
  return screen.getAllByRole('row');
}

describe('QueueTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('affiche une poignée de glissement devant chaque média en attente', () => {
    renderQueue();
    expect(screen.getByRole('button', { name: 'Réordonner Alpha' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Réordonner Charlie' })).toBeTruthy();
  });

  it('envoie la file entière après un glisser-déposer', async () => {
    const rows = renderQueue();
    expect(rows[1].getAttribute('draggable')).toBe('true');
    const setDragImage = vi.fn();
    fireEvent.dragStart(rows[1], {
      dataTransfer: { effectAllowed: '', setData: vi.fn(), setDragImage },
    });
    expect(setDragImage).toHaveBeenCalledWith(rows[1], 16, 16);
    fireEvent.dragOver(rows[3]);
    fireEvent.drop(rows[3]);
    await waitFor(() => {
      expect(mocks.reorderQueue).toHaveBeenCalledWith(guildFixture.id, ['b', 'c', 'a']);
    });
  });

  it('recharge la file quand le serveur la juge périmée', async () => {
    const refresh = vi.fn(async () => {});
    const rows = renderQueue(refresh);
    mocks.reorderQueue.mockRejectedValueOnce(Object.assign(new Error('stale'), { status: 409 }));
    fireEvent.dragStart(rows[1], {
      dataTransfer: { effectAllowed: '', setData: vi.fn(), setDragImage: vi.fn() },
    });
    expect(rows[1].className).toContain('is-dragging');
    fireEvent.dragOver(rows[3]);
    fireEvent.drop(rows[3]);
    await waitFor(() => {
      expect(mocks.reorderQueue).toHaveBeenCalledTimes(1);
      expect(refresh).toHaveBeenCalledTimes(1);
    });
  });
});
