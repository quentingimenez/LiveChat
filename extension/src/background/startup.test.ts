import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { MESSAGE_TYPES, isGetThemePreferenceRequest, isThemePreferenceChangedMessage } from '../lib/messages';

const setAccessLevel = vi.fn().mockResolvedValue(undefined);
const tabsQuery = vi.fn().mockResolvedValue([
  { id: 1, url: 'https://www.youtube.com/watch?v=abc123' },
  { id: 2, url: 'https://example.com/' },
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
    tabsSendMessage.mockClear();
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

  it('pousse le thème aux onglets supportés quand la préférence change', async () => {
    await import('./index');

    storageChangedHandler?.({ 'lce.theme.v1': { newValue: 'system' } }, 'local');

    await vi.waitFor(() => expect(tabsSendMessage).toHaveBeenCalled());
    expect(tabsSendMessage).toHaveBeenCalledTimes(1);
    const pushed = tabsSendMessage.mock.calls[0][1];
    expect(isThemePreferenceChangedMessage(pushed)).toBe(true);
    expect(pushed).toMatchObject({
      type: MESSAGE_TYPES.THEME_PREFERENCE_CHANGED,
      preference: 'system',
    });
  });
});
