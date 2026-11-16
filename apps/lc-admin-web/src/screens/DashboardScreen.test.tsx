import { render, screen } from '@testing-library/preact';
import { describe, expect, it } from 'vitest';
import { DashboardView, healthValue } from './DashboardScreen';
import {
  auditFixture,
  guildDetailFixture,
  guildFixture,
  healthFixture,
  overviewFixture,
} from '../test-fixtures';

describe('DashboardView', () => {
  it('renders the dashboard from an overview fixture', () => {
    render(
      <DashboardView
        overview={overviewFixture}
        health={healthFixture}
        ingestions={[]}
        audit={auditFixture}
        airDetails={{ [guildFixture.id]: guildDetailFixture }}
      />,
    );

    expect(screen.getByRole('heading', { name: 'Tableau de bord' })).toBeTruthy();
    expect(screen.getAllByText('Les Zigotos du Dimanche').length).toBeGreaterThan(0);
    expect(screen.getByText('Base SQLite')).toBeTruthy();
    expect(screen.getByText('v3.4.0 · 6 j 4 h')).toBeTruthy();
    expect(screen.getByText('Le chat qui tombe du canapé')).toBeTruthy();
    expect(screen.getByText('Overlay révoqué')).toBeTruthy();
    expect(screen.getByText('42')).toBeTruthy();
  });

  it('renders the unavailable health state without crashing', () => {
    render(
      <DashboardView
        overview={{
          ...overviewFixture,
          guilds: [],
          totals: {
            guildCount: 0,
            overlayClientCount: 0,
            overlayConnectedCount: 0,
            ingestClientCount: 0,
            pairingCodeCount: 0,
            activeInviteCount: 0,
          },
        }}
        healthUnavailable
        audit={[]}
      />,
    );

    expect(screen.getByText('Indisponible sur ce serveur')).toBeTruthy();
    expect(screen.getByText('Rien à l’antenne pour le moment.')).toBeTruthy();
  });
});

describe('healthValue', () => {
  const check = (id: string, status: string, extra: Partial<Parameters<typeof healthValue>[0]> = {}) =>
    healthValue({ id, status, detail: 'raw server detail', version: null, count: null, ...extra });

  it('shows a short French value instead of the raw server detail', () => {
    expect(check('database', 'ok')).toBe('Accessible');
    expect(check('discord', 'warn')).toBe('Non configuré');
    expect(check('discord', 'error')).toBe('Déconnecté');
    expect(check('ffmpeg', 'ok', { version: '9.0.2-full_build-www.gyan.dev' })).toBe('9.0.2');
    expect(check('ytdlp', 'ok')).toBe('Disponible');
    expect(check('piper', 'error')).toBe('Introuvable');
    expect(check('ingestions', 'ok', { count: 0 })).toBe('Aucun échec sur 1 h');
    expect(check('ingestions', 'warn', { count: 3 })).toBe('3 échecs sur 1 h');
    expect(check('other', 'ok')).toBe('raw server detail');
  });
});
