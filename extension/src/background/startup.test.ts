import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { MESSAGE_TYPES, isGetThemePreferenceRequest, isThemePreferenceChangedMessage } from '../lib/messages';

const setAccessLevel = vi.fn().mockResolvedValue(undefined);
const tabsQuery = vi.fn().mockResolvedValue([
  { id: 1, url: 'https://www.youtube.com/watch?v=abc123' },
  { id: 2, url: 'https://example.com/' },
  { id: 3, url: 'https://youtu.be/abc' },
]);
const tabsSendMessage = vi.fn().mockResolvedValue(undefined);
let runtimeMessageHandler: ((message: unknown, sender: unknown, sendResponse: (r: unknown) => void) => void) | null = null;
let storageChangedHandler: ((changes: Record<string, { newValue?: unknown }>, areaName: string) => void) | null = null;

const installBrowserStub = (): void => {
  const listener = () => undefined;
  runtimeMessageHandler = null;
  storageChangedHandler = null;
  vi.stubGlobal('browser', {
    runtime: {
      onInstalled: { addListener: listener },
      onStartup: { addListener: listener },
      onMessage: {
        addListener: (handler: typeof runtimeMessageHandler) => {
          runtimeMessageHandler = handler;
        },
      },
    },
    webRequest: {
      onCompleted: { addListener: listener },
      onBeforeRedirect: { addListener: listener },
    },
    tabs: {
      onRemoved: { addListener: listener },
      query: tabsQuery,
      sendMessage: tabsSendMessage,
    },
    contextMenus: { onClicked: { addListener: listener }, create: vi.fn() },
    action: { openPopup: vi.fn() },
    storage: {
      onChanged: {
        addListener: (handler: typeof storageChangedHandler) => {
          storageChangedHandler = handler;
        },
      },
      local: {
        get: vi.fn().mockResolvedValue({ 'lce.theme.v1': 'light' }),
        setAccessLevel,
      },
      session: {},
    },
  });
};

describe('démarrage du service worker', () => {
  beforeEach(() => {
    vi.resetModules();
    setAccessLevel.mockClear();
    tabsQuery.mockClear();
    tabsSendMessage.mockReset().mockResolvedValue(undefined);
    installBrowserStub();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('restreint storage.local aux contextes de confiance au chargement du service worker', async () => {
    await import('./index');

    expect(setAccessLevel).toHaveBeenCalledTimes(1);
    expect(setAccessLevel).toHaveBeenCalledWith({ accessLevel: 'TRUSTED_CONTEXTS' });
  });

  it('répond à la demande de thème des content scripts', async () => {
    await import('./index');

    const request = { type: MESSAGE_TYPES.GET_THEME_PREFERENCE };
    expect(isGetThemePreferenceRequest(request)).toBe(true);

    const sendResponse = vi.fn();
    runtimeMessageHandler?.(request, {}, sendResponse);
    await vi.waitFor(() => expect(sendResponse).toHaveBeenCalledWith({ preference: 'light' }));
  });

  it('pousse la préférence à tous les onglets, y compris les hôtes non injectés', async () => {
    await import('./index');

    storageChangedHandler?.({ 'lce.theme.v1': { newValue: 'system' } }, 'local');

    await vi.waitFor(() => expect(tabsSendMessage).toHaveBeenCalledTimes(3));
    // Le manifeste est la seule source de vérité des injections : on ne filtre
    // pas les onglets ici, sinon une page injectée comme youtu.be est oubliée.
    expect(tabsSendMessage.mock.calls.map((call) => call[0])).toEqual([1, 2, 3]);
    for (const call of tabsSendMessage.mock.calls) {
      expect(isThemePreferenceChangedMessage(call[1])).toBe(true);
      expect(call[1]).toMatchObject({
        type: MESSAGE_TYPES.THEME_PREFERENCE_CHANGED,
        preference: 'system',
      });
    }
  });

  it('continue la diffusion quand un onglet sans content script rejette le message', async () => {
    tabsSendMessage.mockImplementation((tabId: number) =>
      tabId === 2 ? Promise.reject(new Error('Receiving end does not exist')) : Promise.resolve(undefined),
    );
    await import('./index');

    storageChangedHandler?.({ 'lce.theme.v1': { newValue: 'dark' } }, 'local');

    await vi.waitFor(() => expect(tabsSendMessage).toHaveBeenCalledTimes(3));
    expect(tabsSendMessage.mock.calls.map((call) => call[0])).toEqual([1, 2, 3]);
  });
});
