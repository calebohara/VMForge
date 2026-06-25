import { Loader2, MemoryStick, RotateCcw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { formatBytes, snapshotDateLabel } from "@/lib/format";
import type { Snapshot } from "@/lib/ipc";

type BusyOp = "take" | "restore" | "delete" | null;

/**
 * Detail pane for the selected snapshot, with Restore / Delete actions. Restore
 * is refused while the VM is live (decision A3/A7) — the button is disabled with
 * an explanatory tooltip. While an op runs the in-flight button shows a spinner
 * and the other actions are disabled (spec §D5).
 */
export function SnapshotDetail({
  snapshot,
  live,
  busyOp,
  onRestore,
  onDelete,
}: {
  snapshot: Snapshot;
  /** Source VM is running/paused — restore is refused. */
  live: boolean;
  busyOp: BusyOp;
  onRestore: (snapshot: Snapshot) => void;
  onDelete: (snapshot: Snapshot) => void;
}) {
  const anyBusy = busyOp != null;
  const restoreDisabled = anyBusy || live;

  const restoreBtn = (
    <Button
      variant="outline"
      disabled={restoreDisabled}
      onClick={() => onRestore(snapshot)}
    >
      {busyOp === "restore" ? (
        <Loader2 className="h-4 w-4 animate-spin" />
      ) : (
        <RotateCcw className="h-4 w-4" />
      )}
      {busyOp === "restore" ? "Restoring…" : "Restore"}
    </Button>
  );

  return (
    <div className="flex h-full flex-col gap-4 p-4">
      <div className="space-y-1">
        <h3 className="truncate text-sm font-semibold" title={snapshot.name}>
          {snapshot.name}
        </h3>
        <p className="text-xs text-muted-foreground">
          {snapshotDateLabel(snapshot.created_at)}
        </p>
      </div>

      <dl className="space-y-2 text-xs">
        <div className="flex items-center justify-between gap-2">
          <dt className="text-muted-foreground">Type</dt>
          <dd>
            {snapshot.has_vm_state ? (
              <Badge variant="secondary" className="gap-1">
                <MemoryStick className="h-3 w-3" /> Disk + memory
              </Badge>
            ) : (
              <Badge variant="outline">Disk only</Badge>
            )}
          </dd>
        </div>
        {snapshot.has_vm_state && (
          <div className="flex items-center justify-between gap-2">
            <dt className="text-muted-foreground">Memory captured</dt>
            <dd>{formatBytes(snapshot.vm_state_size)}</dd>
          </div>
        )}
        {!snapshot.present_in_qcow2 && (
          <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2.5 py-1.5 text-amber-600 dark:text-amber-500">
            This snapshot is recorded in your library but missing from the disk
            image. It can be removed but not restored.
          </div>
        )}
      </dl>

      <div className="mt-auto flex flex-col gap-2 border-t border-border pt-4">
        {live ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="inline-flex">{restoreBtn}</span>
            </TooltipTrigger>
            <TooltipContent>
              Stop the VM to restore a snapshot.
            </TooltipContent>
          </Tooltip>
        ) : (
          restoreBtn
        )}
        <Button
          variant="outline"
          className="text-destructive hover:text-destructive"
          disabled={anyBusy}
          onClick={() => onDelete(snapshot)}
        >
          {busyOp === "delete" ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Trash2 className="h-4 w-4" />
          )}
          {busyOp === "delete" ? "Deleting…" : "Delete"}
        </Button>
      </div>
    </div>
  );
}
