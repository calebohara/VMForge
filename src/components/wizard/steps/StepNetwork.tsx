import { NetworkForm } from "@/components/common/NetworkForm";
import type { NetworkConfig } from "@/lib/ipc";

/**
 * Step 4 — Network. Pick the adapter mode, set an optional MAC, and (in NAT /
 * user mode) define port forwards. Wraps the shared {@link NetworkForm}.
 * Bridged / host-only are disabled with the host capability reason.
 */
export function StepNetwork({
  value,
  onChange,
  onValidityChange,
}: {
  value: NetworkConfig;
  onChange: (next: NetworkConfig) => void;
  onValidityChange?: (valid: boolean) => void;
}) {
  return (
    <NetworkForm
      idPrefix="wizard-net"
      variant="wizard"
      value={value}
      onChange={onChange}
      onValidityChange={onValidityChange}
    />
  );
}
