import { Loader2 } from "lucide-react";

/**
 * Full-screen placeholder shown while the initial `probe_host` is in flight
 * (App.tsx early-return, before the view machine). Intentionally minimal: the
 * probe is fast, this just avoids a flash of the gate/library mid-detection.
 */
export function HostProbeLoading() {
  return (
    <div
      className="flex h-full flex-col items-center justify-center gap-3 bg-background text-muted-foreground"
      role="status"
      aria-live="polite"
    >
      <Loader2 className="h-6 w-6 animate-spin" />
      <p className="text-sm">Detecting host capabilities…</p>
    </div>
  );
}
