const { contextBridge, ipcRenderer } = require('electron');

const wrapListener = (channel, callback) => {
  const listener = (_event, payload) => {
    callback(payload);
  };

  ipcRenderer.on(channel, listener);

  return () => {
    ipcRenderer.removeListener(channel, listener);
  };
};

contextBridge.exposeInMainWorld('livechatOverlay', {
  getConfig: () => ipcRenderer.invoke('overlay:get-config'),
  rendererReady: () => ipcRenderer.send('overlay:renderer-ready'),
  onPlay: (callback) => wrapListener('overlay:play', callback),
  onStop: (callback) => wrapListener('overlay:stop', callback),
  onSettings: (callback) => wrapListener('overlay:settings', callback),
  reportError: (payload) => ipcRenderer.send('overlay:error', payload),
  consumePairing: (payload) => ipcRenderer.invoke('pairing:consume', payload),
});
