import { useCallback, useEffect, useRef, useState } from "react";
import { listSnapshots, type Snapshot } from "@/lib/ipc";

export interface UseSnapshots {
  snapshots: Snapshot[];
  /** True only on the very first load (before any data has arrived). */
  loading: boolean;
  error: string | null;
  /** Re-fetch the snapshot list (call after a take/restore/delete). */
  refresh: () => Promise<void>;
}

/**
 * On-demand snapshot fetch for a single VM (spec §D3). Unlike `useVmLibrary`
 * this is NOT polled — the snapshot tree is read on entering the view and after
 * each mutation. A `mountedRef` cancels late results after unmount so we never
 * `setState` on an unmounted component.
 */
export function useSnapshots(vmId: string): UseSnapshots {
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const mountedRef = useRef(true);

  const fetchOnce = useCallback(async () => {
    try {
      const items = await listSnapshots(vmId);
      if (!mountedRef.current) return;
      setSnapshots(items);
      setError(null);
    } catch (e) {
      if (!mountedRef.current) return;
      setError(String(e));
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [vmId]);

  const refresh = useCallback(async () => {
    await fetchOnce();
  }, [fetchOnce]);

  useEffect(() => {
    mountedRef.current = true;
    void fetchOnce();
    return () => {
      mountedRef.current = false;
    };
  }, [fetchOnce]);

  return { snapshots, loading, error, refresh };
}
