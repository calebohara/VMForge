import { useEffect, useState } from "react";
import { Camera, Loader2 } from "lucide-react";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { validateVmName } from "@/lib/validation";

/**
 * Take-snapshot dialog. When the source VM is live the snapshot captures RAM
 * (`has_vm_state`) via a QMP job that holds the VM's monitor for the duration —
 * so while `busy` the dialog stays open, shows a "Saving memory state" spinner,
 * and disables cancel (spec §D5). The name reuses {@link validateVmName} since a
 * snapshot tag must survive the same path/round-trip constraints as a VM name.
 */
export function TakeSnapshotDialog({
  open,
  live,
  busy,
  onCancel,
  onConfirm,
}: {
  open: boolean;
  /** Source VM is running/paused — the snapshot will capture RAM. */
  live: boolean;
  /** The take is in flight (keeps the dialog open with a spinner). */
  busy: boolean;
  onCancel: () => void;
  onConfirm: (name: string) => void;
}) {
  const [name, setName] = useState("");

  // Reset the name each time the dialog opens.
  useEffect(() => {
    if (open) setName("");
  }, [open]);

  const trimmed = name.trim();
  const nameError = trimmed.length === 0 ? null : validateVmName(trimmed);
  const canSubmit = trimmed.length > 0 && nameError === null && !busy;

  return (
    <AlertDialog
      open={open}
      // Block outside-close while busy (cancel is disabled mid-flight).
      onOpenChange={(o) => {
        if (!o && !busy) onCancel();
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Take a snapshot</AlertDialogTitle>
          <AlertDialogDescription>
            {live
              ? "Captures the current disk and memory state. You can restore to this exact point later."
              : "Captures the current disk state. You can restore to this point later."}
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="flex flex-col gap-2">
          <Label htmlFor="snapshot-name">Snapshot name</Label>
          <Input
            id="snapshot-name"
            value={name}
            disabled={busy}
            autoFocus
            placeholder="e.g. Before update"
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && canSubmit) onConfirm(trimmed);
            }}
          />
          {nameError && <p className="text-xs text-destructive">{nameError}</p>}
        </div>

        {busy && live && (
          <div className="flex items-center gap-2 rounded-md border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
            <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
            Saving memory state — this can take a while.
          </div>
        )}

        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            disabled={!canSubmit}
            // Keep the dialog open while the job runs (parent flips `busy`).
            onClick={(e) => {
              e.preventDefault();
              if (canSubmit) onConfirm(trimmed);
            }}
          >
            {busy ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Camera className="h-4 w-4" />
            )}
            {busy ? "Taking…" : "Take snapshot"}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
