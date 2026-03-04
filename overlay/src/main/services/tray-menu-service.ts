import type { MenuItem, MenuItemConstructorOptions } from 'electron';
import type { OverlayRuntimeConfig } from './config-service';
import type { DisplayLike } from './display-service';

interface CreateTrayMenuTemplateOptions {
  config: OverlayRuntimeConfig;
  displays: DisplayLike[];
  selectedDisplay: DisplayLike;
  selectedDisplayIndex: number;
  autoStartSupported: boolean;
  autoUpdateReason: string;
  autoUpdateStateLabel: string;
  connectionStateLabel: string;
  connectionReason: string;
  appVersion: string;
  canOpenBoard: boolean;
  buildVolumeMenuItems: (currentVolume: number) => MenuItemConstructorOptions[];
  formatDisplayMenuLabel: (display: DisplayLike, index: number) => string;
  onToggleEnabled: (checked: boolean) => void;
  onToggleAutoStart: (checked: boolean) => void;
  onToggleShowText: (checked: boolean) => void;
  onOpenPairing: () => void;
  onOpenBoard: () => void;
  onMoveOverlayToDisplay: (display: DisplayLike, index: number) => void;
  onQuit: () => void;
}

function toChecked(menuItem: MenuItem): boolean {
  return menuItem.checked === true;
}

export function createTrayMenuTemplate(options: CreateTrayMenuTemplateOptions): MenuItemConstructorOptions[] {
  const {
    config,
    displays,
    selectedDisplay,
    selectedDisplayIndex,
    autoStartSupported,
    autoUpdateReason,
    autoUpdateStateLabel,
    connectionStateLabel,
    connectionReason,
    appVersion,
    canOpenBoard,
    buildVolumeMenuItems,
    formatDisplayMenuLabel,
    onToggleEnabled,
    onToggleAutoStart,
    onToggleShowText,
    onOpenPairing,
    onOpenBoard,
    onMoveOverlayToDisplay,
    onQuit
  } = options;

  const template: MenuItemConstructorOptions[] = [
    {
      label: `Statut connexion: ${connectionStateLabel}`,
      enabled: false
    },
    ...(connectionReason
      ? [
          {
            label: `Raison: ${connectionReason}`,
            enabled: false
          }
        ]
      : []),
    {
      label: `Mise à jour: ${autoUpdateStateLabel}`,
      enabled: false
    },
    ...(autoUpdateReason
      ? [
          {
            label: `Info MAJ: ${autoUpdateReason}`,
            enabled: false
          }
        ]
      : []),
    {
      label: `Version: v${appVersion}`,
      enabled: false
    },
    { type: 'separator' },
    {
      label: 'Overlay activé',
      type: 'checkbox',
      checked: config.enabled,
      click: (menuItem) => {
        onToggleEnabled(toChecked(menuItem));
      }
    },
    {
      label: autoStartSupported ? 'Lancer au démarrage' : 'Lancer au démarrage (non supporté)',
      type: 'checkbox',
      checked: autoStartSupported && config.autoStart === true,
      enabled: autoStartSupported,
      click: (menuItem) => {
        onToggleAutoStart(toChecked(menuItem));
      }
    },
    {
      label: 'Afficher le texte',
      type: 'checkbox',
      checked: config.showText,
      click: (menuItem) => {
        onToggleShowText(toChecked(menuItem));
      }
    },
    { type: 'separator' },
    {
      label: 'Appairer Overlay',
      click: () => {
        onOpenPairing();
      }
    },
    {
      label: 'Ouvrir Meme Board',
      enabled: canOpenBoard,
      click: () => {
        onOpenBoard();
      }
    },
    { type: 'separator' },
    { label: 'CONFIGURATION ÉCRAN', enabled: false },
    {
      label:
        selectedDisplayIndex >= 0
          ? `Écran sélectionné: ${formatDisplayMenuLabel(selectedDisplay, selectedDisplayIndex)}`
          : 'Écran sélectionné: inconnu',
      enabled: false
    },
    { type: 'separator' }
  ];

  for (const [index, display] of displays.entries()) {
    template.push({
      label: formatDisplayMenuLabel(display, index),
      type: 'radio',
      checked: selectedDisplay.id === display.id,
      click: () => {
        onMoveOverlayToDisplay(display, index);
      }
    });
  }

  template.push({ type: 'separator' });
  template.push({
    label: 'Volume Audio',
    submenu: buildVolumeMenuItems(config.volume)
  });
  template.push({ type: 'separator' });
  template.push({
    label: 'Quitter',
    click: () => {
      onQuit();
    }
  });

  return template;
}
