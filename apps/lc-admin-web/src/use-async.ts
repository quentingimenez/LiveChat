import { useCallback, useEffect, useRef, useState } from 'preact/hooks';

export interface AsyncState<T> {
  data: T | null;
  error: unknown;
  loading: boolean;
  reload: () => void;
  setData: (data: T | null) => void;
}

/**
 * Small fetch hook with cancellation. Loaders receive an AbortSignal so the
 * API client can cancel the request when the route changes.
 */
export function useAsync<T>(
  loader: (signal: AbortSignal) => Promise<T>,
  deps: ReadonlyArray<unknown>,
): AsyncState<T> {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [version, setVersion] = useState(0);
  const loaderRef = useRef(loader);
  loaderRef.current = loader;

  useEffect(() => {
    const controller = new AbortController();
    let active = true;
    setLoading(true);
    setError(null);

    Promise.resolve()
      .then(() => loaderRef.current(controller.signal))
      .then((result) => {
        if (active && !controller.signal.aborted) {
          setData(result);
          setLoading(false);
        }
      })
      .catch((caught: unknown) => {
        if (!active || controller.signal.aborted) {
          return;
        }
        if (caught instanceof DOMException && caught.name === 'AbortError') {
          return;
        }
        setError(caught);
        setLoading(false);
      });

    return () => {
      active = false;
      controller.abort();
    };
    // The caller owns the dependency list; `version` powers reload().
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, version]);

  const reload = useCallback(() => {
    setVersion((current) => current + 1);
  }, []);

  return { data, error, loading, reload, setData };
}
