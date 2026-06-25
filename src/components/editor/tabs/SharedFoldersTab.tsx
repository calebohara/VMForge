import { useEffect, useMemo } from "react";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { SharedFolderRow } from "@/components/common/SharedFolderRow";
import type { SharedFolder } from "@/lib/ipc";
import { MAX_SHARED_FOLDERS, validateSharedFolders } from "@/lib/validation";

/** A blank, writable shared folder, ready to edit. */
function emptyFolder(): SharedFolder {
  return { host_path: "", mount_tag: "", read_only: false };
}

/**
 * Editor tab — virtio-9p shared folders (Phase 5). Controlled like
 * {@link NetworkForm}: the parent owns the list and validity flows up via
 * {@link onValidityChange}. Changes apply at next launch.
 *
 * `onValidityChange` reports whether the list is valid to SAVE: each row needs a
 * host path + a valid, unique mount tag, and the count must be within the cap.
 */
export function SharedFoldersTab({
  value,
  onChange,
  onValidityChange,
  disabled,
  idPrefix = "edit-share",
}: {
  value: SharedFolder[];
  onChange: (next: SharedFolder[]) => void;
  onValidityChange?: (valid: boolean) => void;
  disabled?: boolean;
  idPrefix?: string;
}) {
  const rowErrors = useMemo(() => validateSharedFolders(value), [value]);
  const overCap = value.length > MAX_SHARED_FOLDERS;
  const valid = !overCap && rowErrors.every((e) => e == null);

  useEffect(() => {
    onValidityChange?.(valid);
  }, [valid, onValidityChange]);

  const atCap = value.length >= MAX_SHARED_FOLDERS;

  const setFolder = (index: number, next: SharedFolder) =>
    onChange(value.map((sf, i) => (i === index ? next : sf)));
  const addFolder = () => onChange([...value, emptyFolder()]);
  const removeFolder = (index: number) =>
    onChange(value.filter((_, i) => i !== index));

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-between gap-2">
        <div className="flex flex-col">
          <span className="text-sm font-medium">Shared folders</span>
          <span className="text-xs text-muted-foreground">
            Share host directories with the guest over virtio-9p.
          </span>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled || atCap}
          onClick={addFolder}
        >
          <Plus className="h-4 w-4" /> Add folder
        </Button>
      </div>

      {value.length === 0 ? (
        <p className="rounded-md border border-dashed border-border px-3 py-3 text-center text-xs text-muted-foreground">
          No shared folders. Add one to share a host directory with the guest.
        </p>
      ) : (
        <div className="flex flex-col gap-3">
          {value.map((sf, i) => (
            <SharedFolderRow
              key={i}
              index={i}
              idPrefix={idPrefix}
              value={sf}
              disabled={disabled}
              error={rowErrors[i]}
              onChange={(next) => setFolder(i, next)}
              onRemove={() => removeFolder(i)}
            />
          ))}
        </div>
      )}

      {overCap && (
        <p className="text-xs text-destructive">
          At most {MAX_SHARED_FOLDERS} shared folders are allowed.
        </p>
      )}

      {value.length > 0 && (
        <div className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
          <p className="font-medium text-foreground">Mounting in the guest</p>
          <p className="mt-1">
            Inside a Linux/Unix guest, mount a share by its tag:
          </p>
          <pre className="mt-1 overflow-x-auto rounded bg-background px-2 py-1 font-mono text-[11px]">
            mount -t 9p -o trans=virtio,version=9p2000.L &lt;tag&gt; /mnt/shared
          </pre>
          <p className="mt-1">Linux/Unix guests only.</p>
        </div>
      )}

      <p className="text-xs text-muted-foreground">
        Shared-folder changes apply the next time the VM launches.
      </p>
    </div>
  );
}
