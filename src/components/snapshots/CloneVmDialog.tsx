import { useEffect, useState } from "react";
import { Copy, Loader2 } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import type { CloneKind } from "@/lib/ipc";
import { validateVmName } from "@/lib/validation";

/**
 * Clone dialog (spec §D5). Full = deep, flattened copy (independent disk).
 * Linked = copy-on-write overlay backed by the source disk — fast and small,
 * but the source can't be deleted or restored while a linked clone depends on
 * it. That consequence is a persistent amber callout shown whenever "Linked" is
 * selected. While `busy` the footer is disabled and the action reads "Cloning…".
 */
export function CloneVmDialog({
  open,
  sourceName,
  busy,
  onCancel,
  onConfirm,
}: {
  open: boolean;
  sourceName: string;
  busy: boolean;
  onCancel: () => void;
  onConfirm: (newName: string, linked: boolean) => void;
}) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState<CloneKind>("full");

  // Reset on each open; default the name to "<source> clone".
  useEffect(() => {
    if (open) {
      setName(sourceName ? `${sourceName} clone` : "");
      setKind("full");
    }
  }, [open, sourceName]);

  const trimmed = name.trim();
  const nameError = trimmed.length === 0 ? null : validateVmName(trimmed);
  const canSubmit = trimmed.length > 0 && nameError === null && !busy;

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o && !busy) onCancel();
      }}
    >
      <DialogContent showCloseButton={!busy}>
        <DialogHeader>
          <DialogTitle>Clone “{sourceName}”</DialogTitle>
          <DialogDescription>
            Creates a new virtual machine from this one. The clone starts in the
            Defined state.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-2">
          <Label htmlFor="clone-name">New VM name</Label>
          <Input
            id="clone-name"
            value={name}
            disabled={busy}
            autoFocus
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && canSubmit) onConfirm(trimmed, kind === "linked");
            }}
          />
          {nameError && <p className="text-xs text-destructive">{nameError}</p>}
        </div>

        <div className="flex flex-col gap-2">
          <Label>Clone type</Label>
          <RadioGroup
            value={kind}
            onValueChange={(v) => setKind(v as CloneKind)}
            disabled={busy}
            aria-label="Clone type"
          >
            <label
              htmlFor="clone-full"
              className="flex items-start gap-3 rounded-md border border-border p-3 text-sm has-[:checked]:border-primary"
            >
              <RadioGroupItem value="full" id="clone-full" className="mt-0.5" />
              <span>
                <span className="font-medium">Full clone</span>
                <span className="block text-xs text-muted-foreground">
                  A complete, independent copy of the disk. Larger and slower to
                  create, but fully standalone.
                </span>
              </span>
            </label>
            <label
              htmlFor="clone-linked"
              className="flex items-start gap-3 rounded-md border border-border p-3 text-sm has-[:checked]:border-primary"
            >
              <RadioGroupItem value="linked" id="clone-linked" className="mt-0.5" />
              <span>
                <span className="font-medium">Linked clone</span>
                <span className="block text-xs text-muted-foreground">
                  A copy-on-write overlay backed by the source disk. Fast and
                  space-efficient.
                </span>
              </span>
            </label>
          </RadioGroup>
        </div>

        {kind === "linked" && (
          <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-600 dark:text-amber-500">
            A linked clone depends on “{sourceName}”. While this clone exists you
            won't be able to delete or restore snapshots on the source VM.
          </div>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button
            disabled={!canSubmit}
            onClick={() => {
              if (canSubmit) onConfirm(trimmed, kind === "linked");
            }}
          >
            {busy ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Copy className="h-4 w-4" />
            )}
            {busy ? "Cloning…" : "Clone"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
