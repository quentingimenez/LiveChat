import path from 'node:path';
import fs from 'node:fs';

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

interface RuntimeAssetSourcePaths {
  appRoot: string;
  appIconPath: string;
  preloadScriptPath: string;
  overlayHtmlPath: string;
  pairingHtmlPath: string;
  boardHtmlPath: string;
}

function resolveSourcePaths(baseDir: string): RuntimeAssetSourcePaths {
  const appRoot = path.join(baseDir, '../../..');

  return {
    appRoot,
    appIconPath: path.join(appRoot, 'icon.png'),
    preloadScriptPath: path.join(appRoot, 'dist/preload/index.js'),
    overlayHtmlPath: path.join(appRoot, 'renderer/overlay.html'),
    pairingHtmlPath: path.join(appRoot, 'renderer/pairing.html'),
    boardHtmlPath: path.join(appRoot, 'renderer/board.html')
  };
}

function allRequiredSourceFilesExist(sourcePaths: RuntimeAssetSourcePaths): boolean {
  return (
    fs.existsSync(sourcePaths.appIconPath) &&
    fs.existsSync(sourcePaths.preloadScriptPath) &&
    fs.existsSync(sourcePaths.overlayHtmlPath) &&
    fs.existsSync(sourcePaths.pairingHtmlPath) &&
    fs.existsSync(sourcePaths.boardHtmlPath)
  );
}

function copyRuntimeAssetsToStableCache(sourcePaths: RuntimeAssetSourcePaths, userDataPath: string): RuntimeResolvedPaths | null {
  const cacheRoot = path.join(userDataPath, 'runtime-assets');
  const cacheDistRoot = path.join(cacheRoot, 'dist');
  const cacheRendererRoot = path.join(cacheRoot, 'renderer');
  const cacheDistRendererRoot = path.join(cacheDistRoot, 'renderer');
  const cacheDistPreloadRoot = path.join(cacheDistRoot, 'preload');

  const sourceRendererRoot = path.join(sourcePaths.appRoot, 'renderer');
  const sourceDistRendererRoot = path.join(sourcePaths.appRoot, 'dist/renderer');
  const sourceDistPreloadRoot = path.join(sourcePaths.appRoot, 'dist/preload');

  try {
    fs.mkdirSync(cacheRoot, { recursive: true });
    fs.cpSync(sourceRendererRoot, cacheRendererRoot, {
      recursive: true,
      force: true,
      dereference: true
    });
    fs.cpSync(sourceDistRendererRoot, cacheDistRendererRoot, {
      recursive: true,
      force: true,
      dereference: true
    });
    fs.cpSync(sourceDistPreloadRoot, cacheDistPreloadRoot, {
      recursive: true,
      force: true,
      dereference: true
    });
    fs.copyFileSync(sourcePaths.appIconPath, path.join(cacheRoot, 'icon.png'));
  } catch {
    return null;
  }

  const resolvedCachedPaths: RuntimeResolvedPaths = {
    configPath: path.join(userDataPath, 'config.json'),
    appIconPath: path.join(cacheRoot, 'icon.png'),
    preloadScriptPath: path.join(cacheDistPreloadRoot, 'index.js'),
    overlayHtmlPath: path.join(cacheRendererRoot, 'overlay.html'),
    pairingHtmlPath: path.join(cacheRendererRoot, 'pairing.html'),
    boardHtmlPath: path.join(cacheRendererRoot, 'board.html')
  };

  if (
    !fs.existsSync(resolvedCachedPaths.appIconPath) ||
    !fs.existsSync(resolvedCachedPaths.preloadScriptPath) ||
    !fs.existsSync(resolvedCachedPaths.overlayHtmlPath) ||
    !fs.existsSync(resolvedCachedPaths.pairingHtmlPath) ||
    !fs.existsSync(resolvedCachedPaths.boardHtmlPath)
  ) {
    return null;
  }

  return resolvedCachedPaths;
}

export function resolveRuntimePaths(params: ResolveRuntimePathsParams): RuntimeResolvedPaths {
  const sourcePaths = resolveSourcePaths(params.baseDir);
  const fallbackPaths: RuntimeResolvedPaths = {
    configPath: path.join(params.userDataPath, 'config.json'),
    appIconPath: sourcePaths.appIconPath,
    preloadScriptPath: sourcePaths.preloadScriptPath,
    overlayHtmlPath: sourcePaths.overlayHtmlPath,
    pairingHtmlPath: sourcePaths.pairingHtmlPath,
    boardHtmlPath: sourcePaths.boardHtmlPath
  };

  if (!allRequiredSourceFilesExist(sourcePaths)) {
    return fallbackPaths;
  }

  const cachedPaths = copyRuntimeAssetsToStableCache(sourcePaths, params.userDataPath);
  if (!cachedPaths) {
    return fallbackPaths;
  }

  return {
    ...cachedPaths,
    configPath: fallbackPaths.configPath
  };
}
