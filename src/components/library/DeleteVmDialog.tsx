import { useEffect, useState } from "react";
import { Trash2 } from "lucide-react";
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
import { Label } from "@/components/ui/label";
import type { VmListItem } from "@/lib/ipc";

/**
 * Confirmation dialog for deleting a VM. The "also delete disk files" choice
 * maps to the `delete_disks` IPC arg. Controlled via `vm` (null => closed).
 */
export function DeleteVmDialog({
  vm,
  onCancel,
  onConfirm,
}: {
  vm: VmListItem | null;
  onCancel: () => void;
  onConfirm: (id: string, deleteDisks: boolean) => void;
}) {
  const [deleteDisks, setDeleteDisks] = useState(true);

  // Reset the checkbox each time the dialog opens for a new VM.
  useEffect(() => {
    if (vm) setDeleteDisks(true);
  }, [vm]);

  return (
    <AlertDialog open={vm != null} onOpenChange={(open) => !open && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete “{vm?.name}”?</AlertDialogTitle>
          <AlertDialogDescription>
            This removes the virtual machine from your library. This action
            cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>

        <label className="flex items-center gap-2.5 rounded-md border border-border p-3 text-sm">
          <input
            type="checkbox"
            checked={deleteDisks}
            onChange={(e) => setDeleteDisks(e.target.checked)}
            className="h-4 w-4 accent-destructive"
          />
          <span>
            <Label className="font-medium">Also delete disk files</Label>
            <span className="block text-xs text-muted-foreground">
              Permanently removes the VM directory and its qcow2 disks. Leave
              unchecked to keep the disks on disk.
            </span>
          </span>
        </label>

        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            onClick={() => vm && onConfirm(vm.id, deleteDisks)}
          >
            <Trash2 className="h-4 w-4" /> Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
