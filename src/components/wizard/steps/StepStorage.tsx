import { LimitField } from "@/components/common/LimitField";
import {
  MAX_DISK_GIB,
  MIN_DISK_GIB,
  validateDiskGib,
} from "@/lib/validation";

/**
 * Step 3 — Storage. A single qcow2 disk is created at this size (sparse, so the
 * file grows on demand). Adding/resizing disks is out of scope for Phase 2.
 */
export function StepStorage({
  diskGib,
  onDiskChange,
}: {
  diskGib: number;
  onDiskChange: (diskGib: number) => void;
}) {
  return (
    <div className="flex flex-col gap-6">
      <LimitField
        id="vm-disk"
        label="Primary disk"
        unit="GiB"
        value={diskGib}
        min={MIN_DISK_GIB}
        max={Math.min(MAX_DISK_GIB, Math.max(256, diskGib * 2))}
        step={1}
        error={validateDiskGib(diskGib)}
        hint="A single qcow2 disk is created. It grows on demand up to this size."
        onChange={onDiskChange}
      />
      <p className="text-xs text-muted-foreground">
        Additional disks and resizing arrive in a later phase.
      </p>
    </div>
  );
}
