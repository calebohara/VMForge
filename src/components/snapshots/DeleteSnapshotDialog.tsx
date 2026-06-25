import { Loader2, Trash2 } from "lucide-react";
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
 * Delete-confirmation for a single snapshot. Deleting a snapshot with children
 * re-parents them to its parent (engine `reparent_on_delete`); we surface that
 * so the user knows the subtree is preserved, not destroyed. While `busy` the
 * dialog stays open with a spinner and cancel is disabled (spec §D5).
 */
export function DeleteSnapshotDialog({
  snapshot,
  childCount,
  busy,
  onCancel,
  onConfirm,
}: {
  /** The snapshot to delete (null => closed). */
  snapshot: Snapshot | null;
  /** Number of direct children that will be re-parented. */
  childCount: number;
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
          <AlertDialogTitle>Delete “{snapshot?.name}”?</AlertDialogTitle>
          <AlertDialogDescription>
            This permanently removes the snapshot. This action cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>

        {childCount > 0 && (
          <div className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
            {childCount === 1
              ? "Its child snapshot will be re-parented to this snapshot's parent."
              : `Its ${childCount} child snapshots will be re-parented to this snapshot's parent.`}
          </div>
        )}

        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={busy}
            onClick={(e) => {
              e.preventDefault();
              if (snapshot && !busy) onConfirm(snapshot.snapshot_id);
            }}
          >
            {busy ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Trash2 className="h-4 w-4" />
            )}
            {busy ? "Deleting…" : "Delete"}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
