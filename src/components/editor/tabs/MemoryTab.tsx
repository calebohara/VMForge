import { LimitField } from "@/components/common/LimitField";
import { formatMemory } from "@/lib/format";
import {
  MAX_MEMORY_MIB,
  MIN_MEMORY_MIB,
  validateMemoryMib,
} from "@/lib/validation";

/** Editor tab — RAM allocation in MiB. */
export function MemoryTab({
  memoryMib,
  disabled,
  onChange,
}: {
  memoryMib: number;
  disabled?: boolean;
  onChange: (memoryMib: number) => void;
}) {
  const sliderMax = Math.min(MAX_MEMORY_MIB, Math.max(64 * 1024, memoryMib * 2));
  return (
    <LimitField
      id="edit-memory"
      label="Memory"
      unit="MiB"
      value={memoryMib}
      min={MIN_MEMORY_MIB}
      max={sliderMax}
      step={256}
      disabled={disabled}
      error={validateMemoryMib(memoryMib)}
      hint={`${formatMemory(memoryMib)} allocated — keep enough RAM free for the host OS.`}
      onChange={onChange}
    />
  );
}
