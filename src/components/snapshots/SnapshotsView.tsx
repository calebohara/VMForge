import { useCallback, useMemo, useState } from "react";
import { Camera, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { StatusBadge } from "@/components/common/StatusBadge";
import { SnapshotTree } from "@/components/snapshots/SnapshotNode";
import { SnapshotDetail } from "@/components/snapshots/SnapshotDetail";
import { EmptySnapshots } from "@/components/snapshots/EmptySnapshots";
import { TakeSnapshotDialog } from "@/components/snapshots/TakeSnapshotDialog";
import { RestoreSnapshotDialog } from "@/components/snapshots/RestoreSnapshotDialog";
import { DeleteSnapshotDialog } from "@/components/snapshots/DeleteSnapshotDialog";
import { useSnapshots } from "@/hooks/useSnapshots";
import { isLive } from "@/lib/format";
import {
  createSnapshot,
  deleteSnapshot,
  restoreSnapshot,
  type Snapshot,
  type VmState,
} from "@/lib/ipc";

type BusyOp = "take" | "restore" | "delete" | null;

/**
 * Snapshot manager for one VM (spec §D). Left: the snapshot tree + a toolbar.
 * Right: the selected snapshot's detail + actions. All long ops are synchronous
 * (A6): we lock the UI with a `busyOp`, keep the relevant dialog open with a
 * spinner, then refresh-after-success (no optimistic rows) and toast outcomes.
 */
export function SnapshotsView({
  vmId,
  vmName,
  state,
}: {
  vmId: string;
  vmName: string;
  /** Live lifecycle state from the library poll. */
  state: VmState;
}) {
  const { snapshots, loading, refresh } = useSnapshots(vmId);
  const live = isLive(state);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [busyOp, setBusyOp] = useState<BusyOp>(null);

  // Dialog open state.
  const [takeOpen, setTakeOpen] = useState(false);
  const [pendingRestore, setPendingRestore] = useState<Snapshot | null>(null);
  const [pendingDelete, setPendingDelete] = useState<Snapshot | null>(null);

  const selected = useMemo(
    () => snapshots.find((s) => s.snapshot_id === selectedId) ?? null,
    [snapshots, selectedId],
  );

  const deleteChildCount = useMemo(() => {
    if (!pendingDelete) return 0;
    return snapshots.filter((s) => s.parent_id === pendingDelete.snapshot_id)
      .length;
  }, [snapshots, pendingDelete]);

  const onTake = useCallback(
    async (name: string) => {
      setBusyOp("take");
      try {
        await createSnapshot(vmId, name);
        await refresh();
        setTakeOpen(false);
        toast.success(`Snapshot “${name}” taken`);
      } catch (e) {
        toast.error(String(e));
      } finally {
        setBusyOp(null);
      }
    },
    [vmId, refresh],
  );

  const onRestore = useCallback(
    async (snapshotId: string) => {
      setBusyOp("restore");
      try {
        await restoreSnapshot(vmId, snapshotId);
        await refresh();
        setPendingRestore(null);
        toast.success("Snapshot restored");
      } catch (e) {
        toast.error(String(e));
      } finally {
        setBusyOp(null);
      }
    },
    [vmId, refresh],
  );

  const onDelete = useCallback(
    async (snapshotId: string) => {
      setBusyOp("delete");
      try {
        await deleteSnapshot(vmId, snapshotId);
        await refresh();
        setPendingDelete(null);
        if (selectedId === snapshotId) setSelectedId(null);
        toast.success("Snapshot deleted");
      } catch (e) {
        toast.error(String(e));
      } finally {
        setBusyOp(null);
      }
    },
    [vmId, refresh, selectedId],
  );

  const anyBusy = busyOp != null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-3 border-b border-border px-5 py-3">
        <div className="mr-auto min-w-0">
          <h1 className="truncate text-sm font-semibold" title={vmName}>
            Snapshots
          </h1>
          <p className="text-xs text-muted-foreground">{vmName}</p>
        </div>
        <StatusBadge state={state} />
        <Button
          size="sm"
          disabled={anyBusy}
          onClick={() => setTakeOpen(true)}
        >
          {busyOp === "take" ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Camera className="h-4 w-4" />
          )}
          {busyOp === "take" ? "Taking…" : "Take snapshot"}
        </Button>
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-1 gap-4 p-5 lg:grid-cols-[1fr_18rem]">
        <Card className="flex min-h-0 flex-col overflow-hidden p-0">
          <ScrollArea className="min-h-0 flex-1">
            {loading ? (
              <div className="space-y-2 p-4">
                {Array.from({ length: 3 }).map((_, i) => (
                  <Skeleton key={i} className="h-7 w-full rounded-md" />
                ))}
              </div>
            ) : snapshots.length === 0 ? (
              <EmptySnapshots
                onTake={() => setTakeOpen(true)}
                disabled={anyBusy}
              />
            ) : (
              <div className="px-2">
                <SnapshotTree
                  snapshots={snapshots}
                  selectedId={selectedId}
                  disabled={anyBusy}
                  onSelect={setSelectedId}
                />
              </div>
            )}
          </ScrollArea>
        </Card>

        <Card className="flex min-h-0 flex-col overflow-hidden p-0">
          {selected ? (
            <SnapshotDetail
              snapshot={selected}
              live={live}
              busyOp={busyOp}
              onRestore={(s) => setPendingRestore(s)}
              onDelete={(s) => setPendingDelete(s)}
            />
          ) : (
            <div className="flex h-full items-center justify-center p-6 text-center text-xs text-muted-foreground">
              Select a snapshot to see its details.
            </div>
          )}
        </Card>
      </div>

      <TakeSnapshotDialog
        open={takeOpen}
        live={live}
        busy={busyOp === "take"}
        onCancel={() => setTakeOpen(false)}
        onConfirm={(name) => void onTake(name)}
      />

      <RestoreSnapshotDialog
        snapshot={pendingRestore}
        busy={busyOp === "restore"}
        onCancel={() => setPendingRestore(null)}
        onConfirm={(id) => void onRestore(id)}
      />

      <DeleteSnapshotDialog
        snapshot={pendingDelete}
        childCount={deleteChildCount}
        busy={busyOp === "delete"}
        onCancel={() => setPendingDelete(null)}
        onConfirm={(id) => void onDelete(id)}
      />
    </div>
  );
}
