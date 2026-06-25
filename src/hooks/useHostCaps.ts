import { useCallback, useEffect, useRef, useState } from "react";
import { probeHost, type HostCapabilities } from "@/lib/ipc";

export interface UseHostCaps {
  caps: HostCapabilities | null;
  /** True only during the initial mount probe (gate shows a loading screen). */
  loading: boolean;
  /**
   * True while a user-triggered {@link UseHostCaps.refresh} (the gate's
   * Re-check) is in flight. Distinct from `loading` so the gate can show a
   * button spinner without unmounting itself back to the loading screen.
   */
  refreshing: boolean;
  error: string | null;
  /**
   * Re-probe the host on demand (the first-run gate's "Re-check" button, run
   * after the user installs QEMU or pins a directory). Resolves with the fresh
   * caps (or `null` if the probe failed). Never throws — failures surface via
   * `error`. No app restart involved; `probe_host` is idempotent.
   */
  refresh: () => Promise<HostCapabilities | null>;
  /**
   * Best-effort host logical-core count, used only to surface allocation
   * headroom in the wizard/editor. The backend does not expose host topology
   * today, so this falls back to `navigator.hardwareConcurrency` (available in
   * the webview) and is `null` when unknown. Never used to hard-block.
   */
  hostCores: number | null;
}

/** Best-effort host logical-core count from the webview (never throws). */
function detectHostCores(): number | null {
  const n =
    typeof navigator !== "undefined" ? navigator.hardwareConcurrency : undefined;
  return typeof n === "number" && Number.isFinite(n) && n > 0
    ? Math.floor(n)
    : null;
}

/**
 * Probe host virtualization capabilities on mount, and expose a `refresh()` for
 * the first-run gate to re-probe after the user installs QEMU or locates it.
 * The result is otherwise stable for the session (accelerator, headroom,
 * warnings).
 */
export function useHostCaps(): UseHostCaps {
  const [caps, setCaps] = useState<HostCapabilities | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Guards every async setState so a probe that resolves after unmount (or a
  // re-check fired right before navigation away) can't update a dead tree.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const refresh = useCallback(async (): Promise<HostCapabilities | null> => {
    if (mounted.current) setRefreshing(true);
    try {
      const c = await probeHost();
      if (mounted.current) {
        setCaps(c);
        setError(null); // success clears any stale probe error
      }
      return c;
    } catch (e) {
      if (mounted.current) setError(String(e));
      return null;
    } finally {
      if (mounted.current) setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    probeHost()
      .then((c) => {
        if (!cancelled && mounted.current) {
          setCaps(c);
          setError(null);
        }
      })
      .catch((e) => {
        if (!cancelled && mounted.current) setError(String(e));
      })
      .finally(() => {
        if (!cancelled && mounted.current) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return { caps, loading, refreshing, error, refresh, hostCores: detectHostCores() };
}
