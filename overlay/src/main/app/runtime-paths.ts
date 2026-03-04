import path from 'node:path';

interface ResolveRuntimePathsParams {
  userDataPath: string;
  baseDir: string;
}

export interface RuntimeResolvedPaths {
  configPath: string;
  appIconPath: string;
  preloadScriptPath: string;
  overlayHtmlPath: string;
  pairingHtmlPath: string;
  boardHtmlPath: string;
}

export function resolveRuntimePaths(params: ResolveRuntimePathsParams): RuntimeResolvedPaths {
  return {
    configPath: path.join(params.userDataPath, 'config.json'),
    appIconPath: path.join(params.baseDir, '../../../icon.png'),
    preloadScriptPath: path.join(params.baseDir, '../../../preload.js'),
    overlayHtmlPath: path.join(params.baseDir, '../../../renderer/overlay.html'),
    pairingHtmlPath: path.join(params.baseDir, '../../../renderer/pairing.html'),
    boardHtmlPath: path.join(params.baseDir, '../../../renderer/board.html'),
  };
}
