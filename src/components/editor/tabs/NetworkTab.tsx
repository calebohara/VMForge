import { NetworkModeField } from "@/components/common/NetworkModeField";
import type { NetworkMode } from "@/lib/ipc";

/**
 * Editor tab — network adapter mode. Bridged / host-only remain disabled with an
 * explanation (Phase 4).
 */
export function NetworkTab({
  mode,
  disabled,
  onChange,
}: {
  mode: NetworkMode;
  disabled?: boolean;
  onChange: (mode: NetworkMode) => void;
}) {
  return (
    <NetworkModeField
      id="edit-network-mode"
      mode={mode}
      disabled={disabled}
      onChange={onChange}
    />
  );
}
