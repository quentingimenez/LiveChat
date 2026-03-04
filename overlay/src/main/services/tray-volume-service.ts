import type { MenuItemConstructorOptions } from 'electron';

interface VolumePreset {
  label: string;
  value: number;
}

interface BuildVolumeMenuItemsOptions {
  currentVolume: number;
  normalizeVolume: (volume: unknown) => number;
  onVolumeSelected: (volume: number) => void;
}

const VOLUME_PRESETS: ReadonlyArray<VolumePreset> = Object.freeze([
  { label: 'Muet (0%)', value: 0 },
  { label: '10%', value: 0.1 },
  { label: '15%', value: 0.15 },
  { label: '20%', value: 0.2 },
  { label: '25%', value: 0.25 },
  { label: '35%', value: 0.35 },
  { label: '50%', value: 0.5 },
  { label: '65%', value: 0.65 },
  { label: '75%', value: 0.75 },
  { label: '85%', value: 0.85 },
  { label: '100%', value: 1 }
]);

function getClosestVolumePreset(volume: number, normalizeVolume: (volume: unknown) => number): VolumePreset {
  const normalizedVolume = normalizeVolume(volume);
  let selectedPreset = VOLUME_PRESETS[0];

  for (const preset of VOLUME_PRESETS) {
    if (Math.abs(preset.value - normalizedVolume) < Math.abs(selectedPreset.value - normalizedVolume)) {
      selectedPreset = preset;
    }
  }

  return selectedPreset;
}

export function buildVolumeMenuItems(options: BuildVolumeMenuItemsOptions): MenuItemConstructorOptions[] {
  const { currentVolume, normalizeVolume, onVolumeSelected } = options;
  const selectedPreset = getClosestVolumePreset(currentVolume, normalizeVolume);

  return VOLUME_PRESETS.map((preset) => ({
    label: preset.label,
    type: 'radio',
    checked: preset.value === selectedPreset.value,
    click: () => {
      onVolumeSelected(preset.value);
    }
  }));
}
