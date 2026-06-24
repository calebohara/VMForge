import { LimitField } from "@/components/common/LimitField";
import { formatMemory } from "@/lib/format";
import {
  MAX_CPUS,
  MAX_MEMORY_MIB,
  MIN_CPUS,
  MIN_MEMORY_MIB,
  validateCpus,
  validateMemoryMib,
} from "@/lib/validation";

/**
 * Step 2 — CPU & Memory. Sliders + number inputs with honest host headroom
 * captions (logical cores) and a soft over-allocation warning. RAM is capped at
 * a generous slider ceiling; headroom for RAM is informational only because the
 * backend does not expose total host RAM.
 */
export function StepCpuMemory({
  cpus,
  memoryMib,
  hostCores,
  onCpusChange,
  onMemoryChange,
}: {
  cpus: number;
  memoryMib: number;
  hostCores: number | null;
  onCpusChange: (cpus: number) => void;
  onMemoryChange: (memoryMib: number) => void;
}) {
  // Keep the RAM slider usable: cap to 64 GiB or the next power-of-two above the
  // current value, whichever is larger, without exceeding the absolute max.
  const ramSliderMax = Math.min(
    MAX_MEMORY_MIB,
    Math.max(64 * 1024, memoryMib * 2),
  );

  return (
    <div className="flex flex-col gap-6">
      <LimitField
        id="vm-cpus"
        label="Processors"
        unit="vCPUs"
        value={cpus}
        min={MIN_CPUS}
        max={MAX_CPUS}
        step={1}
        softMax={hostCores ?? undefined}
        softMaxLabel={
          hostCores != null ? `Host: ${hostCores} logical cores` : undefined
        }
        error={validateCpus(cpus)}
        onChange={onCpusChange}
      />

      <LimitField
        id="vm-memory"
        label="Memory"
        unit="MiB"
        value={memoryMib}
        min={MIN_MEMORY_MIB}
        max={ramSliderMax}
        step={256}
        error={validateMemoryMib(memoryMib)}
        hint={`${formatMemory(memoryMib)} allocated — keep enough RAM free for the host OS.`}
        onChange={onMemoryChange}
      />
    </div>
  );
}
