import { LimitField } from "@/components/common/LimitField";
import { MAX_CPUS, MIN_CPUS, validateCpus } from "@/lib/validation";

/** Editor tab — vCPU count, with host-core headroom and over-allocation warning. */
export function ProcessorsTab({
  cpus,
  hostCores,
  disabled,
  onChange,
}: {
  cpus: number;
  hostCores: number | null;
  disabled?: boolean;
  onChange: (cpus: number) => void;
}) {
  return (
    <LimitField
      id="edit-cpus"
      label="Processors"
      unit="vCPUs"
      value={cpus}
      min={MIN_CPUS}
      max={MAX_CPUS}
      step={1}
      disabled={disabled}
      softMax={hostCores ?? undefined}
      softMaxLabel={
        hostCores != null ? `Host: ${hostCores} logical cores` : undefined
      }
      error={validateCpus(cpus)}
      onChange={onChange}
    />
  );
}
