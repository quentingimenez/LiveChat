import { useEffect, useMemo, useState } from 'preact/hooks';
import { getAudit, getAuditExportUrl, isUnavailable } from '../api/client';
import type { AuditEntry } from '../api/types';
import { ActivityTable } from '../components/ActivityTable';
import { Layout } from '../components/Layout';
import {
  ErrorNotice,
  Loading,
  PageHead,
  SearchInput,
  Unavailable,
} from '../components/ui';
import { useApp } from '../context';
import { describeError } from '../errors';
import { plural } from '../format';
import { useAsync } from '../use-async';

export function ActivityScreen() {
  const { route, overview, liveAudit, notify } = useApp();
  const [query, setQuery] = useState('');
  const [appliedQuery, setAppliedQuery] = useState('');
  const [older, setOlder] = useState<AuditEntry[]>([]);
  const [loadingMore, setLoadingMore] = useState(false);

  useEffect(() => {
    const timer = window.setTimeout(() => setAppliedQuery(query.trim()), 300);
    return () => window.clearTimeout(timer);
  }, [query]);

  useEffect(() => {
    setOlder([]);
  }, [appliedQuery]);

  const audit = useAsync((signal) => getAudit({ query: appliedQuery, limit: 100 }, signal), [
    appliedQuery,
  ]);

  const entries = useMemo(() => {
    const base = [...(audit.data?.items ?? []), ...older];
    const known = new Set(base.map((entry) => entry.id));
    const live = liveAudit.filter((entry) => !known.has(entry.id));
    return [...live, ...base];
  }, [audit.data, liveAudit, older]);

  const loadMore = async () => {
    const before = audit.data?.nextBefore;
    if (!before || loadingMore) {
      return;
    }
    setLoadingMore(true);
    try {
      const response = await getAudit({ query: appliedQuery, before, limit: 100 });
      setOlder((current) => [...current, ...response.items]);
    } catch (error) {
      notify(describeError(error), 'error');
    } finally {
      setLoadingMore(false);
    }
  };

  let content;
  if (audit.loading && !audit.data) {
    content = <Loading label="Chargement du journal…" />;
  } else if (isUnavailable(audit.error)) {
    content = (
      <Unavailable detail="Le journal consultable (GET /admin/api/audit) n’est pas encore persisté par ce serveur." />
    );
  } else if (audit.error) {
    content = <ErrorNotice error={audit.error} onRetry={audit.reload} />;
  } else {
    content = (
      <>
        <ActivityTable
          entries={entries}
          guilds={overview?.guilds ?? []}
          empty="Aucune entrée ne correspond à cette recherche."
        />
        <p class="table-foot">
          {plural(entries.length, 'entrée', 'entrées')} · conservation 90 jours
          {audit.data?.nextBefore ? (
            <>
              {' · '}
              <button
                type="button"
                class="btn is-small is-quiet"
                onClick={() => void loadMore()}
                disabled={loadingMore}
              >
                {loadingMore ? 'Chargement…' : 'Charger les entrées plus anciennes'}
              </button>
            </>
          ) : null}
        </p>
      </>
    );
  }

  return (
    <Layout route={route} title="Journal" crumbs={[{ label: 'Journal' }]}>
      <PageHead
        title="Journal"
        sub="Chaque action d’administration : qui, quoi, sur quelle guilde, avec quel résultat."
        actions={
          <a class="btn" href={getAuditExportUrl({ query: appliedQuery })} download>
            Exporter en CSV
          </a>
        }
      />
      <div class="toolbar">
        <SearchInput
          value={query}
          onInput={setQuery}
          placeholder="Acteur, action, route ou guilde"
          label="Rechercher dans le journal"
        />
      </div>
      {content}
    </Layout>
  );
}
