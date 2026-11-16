import { useEffect, useState } from 'preact/hooks';

export const GUILD_TABS = [
  'overview',
  'members',
  'devices',
  'access',
  'board',
  'queue',
  'settings',
  'discord',
  'danger',
] as const;

export type GuildTab = (typeof GUILD_TABS)[number];

export type RouteName =
  | 'login'
  | 'dashboard'
  | 'guilds'
  | 'guild'
  | 'devices'
  | 'codes'
  | 'ingest'
  | 'roles'
  | 'media'
  | 'activity'
  | 'settings';

export interface Route {
  name: RouteName;
  guildId?: string;
  tab?: GuildTab;
  query: Record<string, string>;
}

const TOP_LEVEL: ReadonlySet<string> = new Set([
  'dashboard',
  'guilds',
  'devices',
  'codes',
  'ingest',
  'roles',
  'media',
  'activity',
  'settings',
]);

const GUILD_TAB_SET: ReadonlySet<string> = new Set(GUILD_TABS);

function decode(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}

export function isGuildTab(value: string | undefined | null): value is GuildTab {
  return value != null && GUILD_TAB_SET.has(value);
}

/**
 * Parse the hash router used by the mockup:
 * `#/login?error=not_allowed`, `#/guilds/{id}/{tab}`, `#/devices`, …
 * An empty or unknown hash falls back to the dashboard.
 */
export function parseHash(hash: string): Route {
  const raw = hash.startsWith('#') ? hash.slice(1) : hash;
  const queryIndex = raw.indexOf('?');
  const path = queryIndex >= 0 ? raw.slice(0, queryIndex) : raw;
  const queryString = queryIndex >= 0 ? raw.slice(queryIndex + 1) : '';
  const segments = path
    .split('/')
    .filter(Boolean)
    .map(decode);

  const query: Record<string, string> = {};
  if (queryString) {
    new URLSearchParams(queryString).forEach((value, key) => {
      query[key] = value;
    });
  }

  const [first, second, third] = segments;
  if (!first) {
    return { name: 'dashboard', query };
  }
  if (first === 'login') {
    return { name: 'login', query };
  }
  if (first === 'guilds') {
    if (second) {
      return {
        name: 'guild',
        guildId: second,
        tab: isGuildTab(third) ? third : 'overview',
        query,
      };
    }
    return { name: 'guilds', query };
  }
  if (TOP_LEVEL.has(first)) {
    return { name: first as RouteName, query };
  }
  return { name: 'dashboard', query };
}

/** Build a hash URL from a route object (always starts with `#/`). */
export function buildHash(route: Pick<Route, 'name' | 'guildId' | 'tab' | 'query'>): string {
  let path: string;
  if (route.name === 'guild' && route.guildId) {
    path = `/guilds/${encodeURIComponent(route.guildId)}/${route.tab ?? 'overview'}`;
  } else if (route.name === 'guild') {
    path = '/guilds';
  } else if (route.name === 'dashboard') {
    path = '/dashboard';
  } else {
    path = `/${route.name}`;
  }

  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(route.query ?? {})) {
    if (value !== '') {
      params.set(key, value);
    }
  }
  const suffix = params.toString();
  return `#${path}${suffix ? `?${suffix}` : ''}`;
}

export function navigate(route: Pick<Route, 'name' | 'guildId' | 'tab' | 'query'>): void {
  const target = buildHash(route);
  if (window.location.hash !== target) {
    window.location.hash = target;
  }
}

/** Reactive hash router: re-renders on `hashchange`. */
export function useHashRoute(): Route {
  const [route, setRoute] = useState<Route>(() => parseHash(window.location.hash));

  useEffect(() => {
    const onChange = () => setRoute(parseHash(window.location.hash));
    window.addEventListener('hashchange', onChange);
    return () => window.removeEventListener('hashchange', onChange);
  }, []);

  return route;
}
