import { Camera } from "lucide-react";
import { Button } from "@/components/ui/button";

/** Empty state for a VM with no snapshots yet. */
export function EmptySnapshots({
  onTake,
  disabled,
}: {
  onTake: () => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 p-10 text-center">
      <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <Camera className="h-6 w-6" />
      </div>
      <div className="space-y-1">
        <p className="text-sm font-medium">No snapshots yet</p>
        <p className="max-w-xs text-xs text-muted-foreground">
          Snapshots capture a point-in-time you can return to. Take one before a
          risky change.
        </p>
      </div>
      <Button onClick={onTake} disabled={disabled}>
        <Camera className="h-4 w-4" /> Take a snapshot
      </Button>
    </div>
  );
}
