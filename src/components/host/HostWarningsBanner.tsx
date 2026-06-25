import { AlertTriangle } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { hasHostWarnings } from "@/lib/hostStatus";
import type { HostCapabilities } from "@/lib/ipc";

/**
 * Soft, non-blocking host-warnings banner for the top of the library (spec §C).
 * Renders `null` when the host is clean (no engine warnings and hardware
 * acceleration available), so callers can mount it unconditionally. Distinct
 * from the hard {@link QemuRequiredGate}: these are degradations (TCG fallback,
 * emulation notes), not show-stoppers.
 */
export function HostWarningsBanner({
  caps,
}: {
  caps: HostCapabilities | null;
}) {
  if (!caps || !hasHostWarnings(caps)) return null;

  // Surface the engine's own warnings verbatim; if acceleration is unavailable
  // but the probe didn't emit a warning string for it, add an honest fallback.
  const messages =
    caps.warnings.length > 0
      ? caps.warnings
      : [
          "No hardware accelerator available — VMs run under TCG software emulation (expect reduced performance).",
        ];

  return (
    <div className="space-y-2 px-5 pt-4" data-testid="host-warnings-banner">
      {messages.map((w, i) => (
        <Alert key={i} className="border-amber-500/30 bg-amber-500/10">
          <AlertTriangle className="h-4 w-4 text-amber-500" />
          <AlertTitle>Heads up</AlertTitle>
          <AlertDescription>{w}</AlertDescription>
        </Alert>
      ))}
    </div>
  );
}
