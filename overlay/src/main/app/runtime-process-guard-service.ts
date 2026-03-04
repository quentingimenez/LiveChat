interface RuntimeAppLike {
  commandLine: {
    appendSwitch(name: string, value?: string): void;
  };
  setAppUserModelId(appId: string): void;
  requestSingleInstanceLock(): boolean;
  quit(): void;
}

interface ApplyRuntimeProcessGuardsOptions {
  app: RuntimeAppLike;
  platform: string;
  windowsAppUserModelId: string;
  ignoreCertificateErrorsSwitch: string;
  autoplayPolicySwitch: string;
  autoplayPolicyValue: string;
}

export function applyRuntimeProcessGuards(options: ApplyRuntimeProcessGuardsOptions): boolean {
  const {
    app,
    platform,
    windowsAppUserModelId,
    ignoreCertificateErrorsSwitch,
    autoplayPolicySwitch,
    autoplayPolicyValue
  } = options;

  app.commandLine.appendSwitch(ignoreCertificateErrorsSwitch);
  app.commandLine.appendSwitch(autoplayPolicySwitch, autoplayPolicyValue);

  if (platform === 'win32') {
    app.setAppUserModelId(windowsAppUserModelId);
  }

  const hasSingleInstanceLock = app.requestSingleInstanceLock();
  if (!hasSingleInstanceLock) {
    app.quit();
    process.exit(0);
  }

  return hasSingleInstanceLock;
}
