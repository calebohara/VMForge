import { useEffect, useState } from "react";
import {
  networkCapabilities,
  type ModeCapability,
  type NetworkCapabilities,
  type NetworkMode,
} from "@/lib/ipc";

/**
 * Module-level promise cache: the `network_capabilities` probe is launched at
 * most once per app session and every hook consumer shares the result. The
 * capability picture is host-static (it does not change while the app runs).
 */
let cached: Promise<NetworkCapabilities> | null = null;

function probeOnce(): Promise<NetworkCapabilities> {
  if (!cached) {
    cached = networkCapabilities().catch((e) => {
      // Drop the cache on failure so a later mount can retry.
      cached = null;
      throw e;
    });
  }
  return cached;
}

/** Test-only: forget the cached probe so suites start clean. */
export function __resetNetworkCapsCache(): void {
  cached = null;
}

export interface UseNetworkCaps {
  caps: NetworkCapabilities | null;
  loading: boolean;
  error: string | null;
  /** Look up the capability for a single mode (`undefined` until loaded). */
  forMode: (mode: NetworkMode) => ModeCapability | undefined;
}

/**
 * Probe host networking capabilities (which modes are available, why others are
 * not, and whether forwards are loopback-only). Shared, cached, probed once.
 */
export function useNetworkCaps(): UseNetworkCaps {
  const [caps, setCaps] = useState<NetworkCapabilities | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    probeOnce()
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

  const forMode = (mode: NetworkMode): ModeCapability | undefined =>
    caps?.modes.find((m) => m.mode === mode);

  return { caps, loading, error, forMode };
}
