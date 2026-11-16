import type { GuildDetail, GuildEntry } from '../../api/types';
import type { Notify } from '../../context';

export interface GuildTabProps {
  guild: GuildEntry;
  detail: GuildDetail | null;
  detailUnavailable: boolean;
  /** Reload the guild detail and the global overview. */
  refresh: () => Promise<void>;
  notify: Notify;
}
