import { createContext } from 'preact';
import type { ComponentChildren } from 'preact';
import { useCallback, useContext, useEffect, useRef, useState } from 'preact/hooks';
import type { Notify, ToastKind } from '../context';
import { Icon } from './icons';

interface ToastItem {
  id: number;
  message: string;
  kind: ToastKind;
}

const ToastContext = createContext<Notify>(() => {});

export function useToast(): Notify {
  return useContext(ToastContext);
}

const TOAST_DURATION_MS = 3_600;

export function ToastProvider({ children }: { children: ComponentChildren }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const nextId = useRef(1);
  const timers = useRef<number[]>([]);

  const notify = useCallback<Notify>((message, kind = 'ok') => {
    const id = nextId.current;
    nextId.current += 1;
    setItems((current) => [...current, { id, message, kind }]);
    const timer = window.setTimeout(() => {
      setItems((current) => current.filter((item) => item.id !== id));
    }, TOAST_DURATION_MS);
    timers.current.push(timer);
  }, []);

  useEffect(
    () => () => {
      for (const timer of timers.current) {
        window.clearTimeout(timer);
      }
      timers.current = [];
    },
    [],
  );

  return (
    <ToastContext.Provider value={notify}>
      {children}
      <div class="toasts" id="toasts" role="status" aria-live="polite">
        {items.map((item) => (
          <div class={`toast${item.kind === 'error' ? ' is-err' : ''}`} key={item.id}>
            <Icon name={item.kind === 'error' ? 'alert' : 'check'} />
            <span>{item.message}</span>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}
