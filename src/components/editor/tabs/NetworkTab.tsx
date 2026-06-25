import { NetworkForm } from "@/components/common/NetworkForm";
import type { NetworkConfig } from "@/lib/ipc";

/**
 * Editor tab — full network configuration (mode, MAC, NAT port forwards). Wraps
 * the shared {@link NetworkForm}. Changes apply at next launch.
 */
export function NetworkTab({
  value,
  disabled,
  onChange,
  onValidityChange,
}: {
  value: NetworkConfig;
  disabled?: boolean;
  onChange: (next: NetworkConfig) => void;
  onValidityChange?: (valid: boolean) => void;
}) {
  return (
    <NetworkForm
      idPrefix="edit-net"
      variant="editor"
      value={value}
      disabled={disabled}
      onChange={onChange}
      onValidityChange={onValidityChange}
    />
  );
}
