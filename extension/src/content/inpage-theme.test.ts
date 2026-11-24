import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { MESSAGE_TYPES } from '../lib/messages';

const sendMessage = vi.fn().mockResolvedValue({ preference: 'light' });
const addListener = vi.fn();
let themeChangedListener: ((message: unknown) => void) | null = null;

const installBrowserStub = (): void => {
  sendMessage.mockClear();
  addListener.mockClear();
  themeChangedListener = null;
  addListener.mockImplementation((listener: (message: unknown) => void) => {
    themeChangedListener = listener;
  });
  vi.stubGlobal('browser', {
    runtime: {
      sendMessage,
      onMessage: { addListener },
    },
  });
};

const importFresh = async () => {
  vi.resetModules();
  return import('./inpage-theme');
};

describe('thème des widgets injectés', () => {
  beforeEach(() => {
    installBrowserStub();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('ne lit pas storage.local et demande la préférence au service worker', async () => {
    const get = vi.fn();
    vi.stubGlobal('document', {
      documentElement: { setAttribute: vi.fn() },
      getElementById: vi.fn().mockReturnValue({}),
      head: {},
    });
    vi.stubGlobal('browser', {
      runtime: { sendMessage, onMessage: { addListener } },
      storage: { local: { get } },
    });

    const { initInpageTheme } = await importFresh();
    initInpageTheme();
    await vi.waitFor(() => expect(sendMessage).toHaveBeenCalled());

    expect(sendMessage).toHaveBeenCalledWith({ type: MESSAGE_TYPES.GET_THEME_PREFERENCE });
    expect(get).not.toHaveBeenCalled();
  });

  it('applique la préférence poussée par le service worker', async () => {
    const setAttribute = vi.fn();
    vi.stubGlobal('document', {
      documentElement: { setAttribute },
      getElementById: vi.fn().mockReturnValue({}),
      head: {},
    });

    const { initInpageTheme } = await importFresh();
    initInpageTheme();
    await vi.waitFor(() => expect(themeChangedListener).not.toBeNull());

    themeChangedListener?.({
      type: MESSAGE_TYPES.THEME_PREFERENCE_CHANGED,
      preference: 'light',
    });

    expect(setAttribute).toHaveBeenCalledWith('data-lce-theme', 'light');
  });
});
