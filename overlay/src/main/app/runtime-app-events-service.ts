interface AppLike {
  on(event: string, listener: () => void): void;
  whenReady(): Promise<void>;
}

interface RegisterRuntimeAppEventsOptions {
  app: AppLike;
  hasSingleInstanceLock: boolean;
  onSecondInstance: () => void;
  onWhenReady: () => Promise<void>;
  onActivate: () => void;
  onBeforeQuitForUpdate: () => void;
  onWillQuit: () => void;
}

export function registerRuntimeAppEvents(options: RegisterRuntimeAppEventsOptions): void {
  const {
    app,
    hasSingleInstanceLock,
    onSecondInstance,
    onWhenReady,
    onActivate,
    onBeforeQuitForUpdate,
    onWillQuit
  } = options;

  if (hasSingleInstanceLock) {
    app.on('second-instance', onSecondInstance);
  }

  app.whenReady().then(async () => {
    await onWhenReady();
    app.on('activate', onActivate);
  });

  app.on('window-all-closed', () => {
    // Keep app running in tray.
  });

  app.on('before-quit-for-update', onBeforeQuitForUpdate);
  app.on('will-quit', onWillQuit);
}
