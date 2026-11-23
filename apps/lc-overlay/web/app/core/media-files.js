// Which files a paste brings to the board. A media copied in the Windows file
// explorer, or an image copied from a page, arrives as a file and may prefill
// the add-media dialog; text, documents and directories keep their native paste
// meaning.

const MEDIA_TYPE = /^(?:image|video|audio)\//i;

// A directory arrives as a zero-byte file without a type. A media whose type
// the WebView does not know still arrives with its size, so only the size tells
// the two apart.
export function isPastedMedia(file) {
  if (!file || !(file.size > 0)) {
    return false;
  }
  return !file.type || MEDIA_TYPE.test(file.type);
}

export function pastedMediaFiles(dataTransfer) {
  const files = dataTransfer && dataTransfer.files ? Array.from(dataTransfer.files) : [];
  return files.filter(isPastedMedia);
}
