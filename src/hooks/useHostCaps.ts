import { useEffect, useState } from "react";
import { probeHost, type HostCapabilities } from "@/lib/ipc";

export interface UseHostCaps {
  caps: HostCapabilities | null;
  loading: boolean;
  error: string | null;
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
 * Probe host virtualization capabilities exactly once on mount. The result is
 * stable for the session (accelerator, headroom, warnings).
 */
export function useHostCaps(): UseHostCaps {
  const [caps, setCaps] = useState<HostCapabilities | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    probeHost()
      .then((c) => {
        if (!cancelled) setCaps(c);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return { caps, loading, error, hostCores: detectHostCores() };
}
