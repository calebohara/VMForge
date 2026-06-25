import { useState } from "react";
import { AlertTriangle, Cpu, MemoryStick, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { VmCard } from "@/components/library/VmCard";
import { EmptyLibrary } from "@/components/library/EmptyLibrary";
import { DeleteVmDialog } from "@/components/library/DeleteVmDialog";
import { CloneVmDialog } from "@/components/snapshots/CloneVmDialog";
import type { VmActions } from "@/components/library/QuickActions";
import type { HostCapabilities, VmListItem } from "@/lib/ipc";

export interface LibraryViewProps {
  vms: VmListItem[];
  loading: boolean;
  caps: HostCapabilities | null;
  busyIds: Set<string>;
  actions: VmActions;
  onCreate: () => void;
  /** Called when delete is confirmed in the dialog. */
  onConfirmDelete: (id: string, deleteDisks: boolean) => void;
  /**
   * Called when a clone is confirmed. Resolves on completion so the dialog can
   * keep its "Cloning…" spinner up for the full synchronous op (spec §D5).
   */
  onConfirmClone: (id: string, newName: string, linked: boolean) => Promise<void>;
}

/**
 * The VM library dashboard: host-headroom banner, a "New VM" button, and a grid
 * of {@link VmCard}s. Owns the delete-confirmation dialog (delete in
 * {@link QuickActions} just opens it; confirmation bubbles up to the parent).
 */
export function LibraryView({
  vms,
  loading,
  caps,
  busyIds,
  actions,
  onCreate,
  onConfirmDelete,
  onConfirmClone,
}: LibraryViewProps) {
  const [pendingDelete, setPendingDelete] = useState<VmListItem | null>(null);
  const [pendingClone, setPendingClone] = useState<VmListItem | null>(null);
  const [cloning, setCloning] = useState(false);

  // Intercept delete + clone to open the local dialogs.
  const wrappedActions: VmActions = {
    ...actions,
    onDelete: (vm) => setPendingDelete(vm),
    onClone: (vm) => setPendingClone(vm),
  };

  const confirmClone = async (newName: string, linked: boolean) => {
    if (!pendingClone) return;
    setCloning(true);
    try {
      await onConfirmClone(pendingClone.id, newName, linked);
      setPendingClone(null);
    } finally {
      setCloning(false);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-between gap-4 border-b border-border px-5 py-3">
        <div className="min-w-0">
          <h1 className="text-sm font-semibold">Library</h1>
          {caps && (
            <p className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
              <span className="flex items-center gap-1">
                <Cpu className="h-3 w-3" /> Host: {caps.arch}
              </span>
              <span className="flex items-center gap-1">
                <MemoryStick className="h-3 w-3" /> {caps.os}
              </span>
            </p>
          )}
        </div>
        <Button onClick={onCreate}>
          <Plus className="h-4 w-4" /> New VM
        </Button>
      </div>

      {caps?.warnings && caps.warnings.length > 0 && (
        <div className="space-y-2 px-5 pt-4">
          {caps.warnings.map((w, i) => (
            <Alert key={i} className="border-amber-500/30 bg-amber-500/10">
              <AlertTriangle className="h-4 w-4 text-amber-500" />
              <AlertTitle>Heads up</AlertTitle>
              <AlertDescription>{w}</AlertDescription>
            </Alert>
          ))}
        </div>
      )}

      <ScrollArea className="min-h-0 flex-1">
        {loading ? (
          <div className="grid grid-cols-1 gap-4 p-5 sm:grid-cols-2 xl:grid-cols-3">
            {Array.from({ length: 3 }).map((_, i) => (
              <Skeleton key={i} className="h-44 w-full rounded-xl" />
            ))}
          </div>
        ) : vms.length === 0 ? (
          <EmptyLibrary onCreate={onCreate} />
        ) : (
          <div className="grid grid-cols-1 gap-4 p-5 sm:grid-cols-2 xl:grid-cols-3">
            {vms.map((vm) => (
              <VmCard
                key={vm.id}
                vm={vm}
                actions={wrappedActions}
                busy={busyIds.has(vm.id)}
              />
            ))}
          </div>
        )}
      </ScrollArea>

      <DeleteVmDialog
        vm={pendingDelete}
        onCancel={() => setPendingDelete(null)}
        onConfirm={(id, deleteDisks) => {
          setPendingDelete(null);
          onConfirmDelete(id, deleteDisks);
        }}
      />

      <CloneVmDialog
        open={pendingClone != null}
        sourceName={pendingClone?.name ?? ""}
        busy={cloning}
        onCancel={() => setPendingClone(null)}
        onConfirm={(newName, linked) =>
          // confirmClone rethrows on failure (to keep the dialog open); the
          // error is already toasted upstream, so swallow it here to avoid an
          // unhandled promise rejection.
          void confirmClone(newName, linked).catch(() => {})
        }
      />
    </div>
  );
}
