import { Loader2, RotateCcw } from "lucide-react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import type { Snapshot } from "@/lib/ipc";

/**
 * Restore-confirmation dialog. Phase 3 restore reverts the DISK ONLY
 * (`qemu-img snapshot -a`); it does not revert RAM or `vmforge.toml` (decision
 * A7) — this is spelled out in the body. Restore is refused while the VM is
 * live, so callers only open this for a stopped VM. While `busy` the dialog
 * stays open with a spinner and cancel is disabled (spec §D5).
 */
export function RestoreSnapshotDialog({
  snapshot,
  busy,
  onCancel,
  onConfirm,
}: {
  /** The snapshot to restore (null => closed). */
  snapshot: Snapshot | null;
  busy: boolean;
  onCancel: () => void;
  onConfirm: (snapshotId: string) => void;
}) {
  return (
    <AlertDialog
      open={snapshot != null}
      onOpenChange={(o) => {
        if (!o && !busy) onCancel();
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Restore “{snapshot?.name}”?</AlertDialogTitle>
          <AlertDialogDescription>
            This reverts the VM's disk to this snapshot. Any changes made since
            then are discarded — this cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-600 dark:text-amber-500">
          Restore reverts the disk only. Saved memory state is not restored in
          this version — the VM will boot fresh from the restored disk.
        </div>

        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            disabled={busy}
            onClick={(e) => {
              e.preventDefault();
              if (snapshot && !busy) onConfirm(snapshot.snapshot_id);
            }}
          >
            {busy ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <RotateCcw className="h-4 w-4" />
            )}
            {busy ? "Restoring…" : "Restore"}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
