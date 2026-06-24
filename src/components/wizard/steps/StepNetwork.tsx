import { NetworkModeField } from "@/components/common/NetworkModeField";
import type { NetworkMode } from "@/lib/ipc";

/**
 * Step 4 — Network. Pick the adapter mode. Bridged / host-only are disabled with
 * an explanation (Phase 4); NAT (user mode) is the default.
 */
export function StepNetwork({
  mode,
  onModeChange,
}: {
  mode: NetworkMode;
  onModeChange: (mode: NetworkMode) => void;
}) {
  return <NetworkModeField mode={mode} onChange={onModeChange} />;
}
