// Signed shortcuts shipped in ios/ (docs/ios-share-shortcut.md). The board runs
// on the PC and cannot import an iOS shortcut by itself: each shortcut button
// copies the URL the member opens from the iPhone.

const SHORTCUT_BASE_URL = 'https://raw.githubusercontent.com/quentingimenez/LiveChat/main/ios/';

export const IOS_SHORTCUTS = Object.freeze([
  Object.freeze({ label: 'Photo', file: 'LiveChat-Photo.shortcut' }),
  Object.freeze({ label: 'Vidéo', file: 'LiveChat-Video.shortcut' }),
  Object.freeze({ label: 'Lien', file: 'LiveChat-Lien.shortcut' }),
]);

export function iosShortcutUrl(shortcut) {
  return `${SHORTCUT_BASE_URL}${shortcut.file}`;
}

export function iosShortcutCopyMessage(shortcut) {
  return `Lien du raccourci ${shortcut.label} copié. Ouvre-le sur l’iPhone pour l’importer.`;
}
