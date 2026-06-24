import { useCallback, useEffect, useRef, useState } from "react";
import { listVms, type VmListItem } from "@/lib/ipc";

const POLL_INTERVAL_MS = 2000;

export interface UseVmLibrary {
  vms: VmListItem[];
  /** True only on the very first load (before any data has arrived). */
  loading: boolean;
  error: string | null;
  /** Force an immediate re-fetch (call after mutating actions). */
  refresh: () => Promise<void>;
}

/**
 * Polls `list_vms` on a 2000ms interval (Phase 2 uses polling, not events).
 * The poll is paused while the document is hidden (`visibilitychange`) and
 * resumes — with an immediate fetch — when the tab becomes visible again.
 */
export function useVmLibrary(): UseVmLibrary {
  const [vms, setVms] = useState<VmListItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Track mount + initial-load state without retriggering effects.
  const mountedRef = useRef(true);
  const loadedRef = useRef(false);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const fetchOnce = useCallback(async () => {
    try {
      const items = await listVms();
      if (!mountedRef.current) return;
      setVms(items);
      setError(null);
    } catch (e) {
      if (!mountedRef.current) return;
      setError(String(e));
    } finally {
      if (mountedRef.current && !loadedRef.current) {
        loadedRef.current = true;
        setLoading(false);
      }
    }
  }, []);

  const refresh = useCallback(async () => {
    await fetchOnce();
  }, [fetchOnce]);

  useEffect(() => {
    mountedRef.current = true;

    const stop = () => {
      if (timerRef.current != null) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };

    const start = () => {
      if (timerRef.current != null) return;
      timerRef.current = setInterval(() => {
        void fetchOnce();
      }, POLL_INTERVAL_MS);
    };

    const onVisibility = () => {
      if (document.hidden) {
        stop();
      } else {
        void fetchOnce();
        start();
      }
    };

    // Initial fetch + start polling (unless we boot up hidden).
    void fetchOnce();
    if (!document.hidden) start();

    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      mountedRef.current = false;
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [fetchOnce]);

  return { vms, loading, error, refresh };
}
