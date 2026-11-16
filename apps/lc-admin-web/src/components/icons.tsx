const ICON_NAMES = [
  'logo',
  'dashboard',
  'guilds',
  'device',
  'plug',
  'key',
  'disk',
  'log',
  'settings',
  'search',
  'plus',
  'x',
  'check',
  'minus',
  'copy',
  'stop',
  'play',
  'shield',
  'external',
  'info',
  'alert',
  'logout',
  'star',
  'chevron',
  'refresh',
  'trash',
  'discord',
] as const;

export type IconName = (typeof ICON_NAMES)[number];

export interface IconProps {
  name: IconName;
  className?: string;
}

/** References a symbol of the sprite inlined in index.html. */
export function Icon({ name, className }: IconProps) {
  return (
    <svg class={`ic${className ? ` ${className}` : ''}`} aria-hidden="true" focusable="false">
      <use href={`#i-${name}`} />
    </svg>
  );
}
