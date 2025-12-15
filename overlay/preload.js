const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('electron', {
  getTikTokVideoUrl: (url) => ipcRenderer.invoke('get-tiktok-video', url)
});
