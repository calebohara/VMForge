import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { DirectoryPicker } from "@/components/common/DirectoryPicker";
import { cn } from "@/lib/utils";
import type { SharedFolder } from "@/lib/ipc";

/**
 * A single shared-folder row (Phase 5): a host-directory picker, the 9p mount
 * tag, a read-only toggle, and a remove button. Fully controlled. Inline
 * blocking errors (empty path / bad-or-duplicate tag) are passed in via
 * {@link error}.
 */
export function SharedFolderRow({
  value,
  index,
  idPrefix = "sf",
  disabled,
  error,
  onChange,
  onRemove,
}: {
  value: SharedFolder;
  /** Zero-based row index, used to build stable input ids and aria-labels. */
  index: number;
  idPrefix?: string;
  disabled?: boolean;
  /** Inline blocking error for this row (empty path / tag / duplicate). */
  error?: string | null;
  onChange: (next: SharedFolder) => void;
  onRemove: () => void;
}) {
  const rowId = `${idPrefix}-row-${index}`;
  const pathId = `${rowId}-path`;
  const tagId = `${rowId}-tag`;
  const invalid = error != null;

  return (
    <div className="flex flex-col gap-2 rounded-md border border-border p-3">
      <div className="flex items-start justify-between gap-2">
        <div className="flex flex-1 flex-col gap-1">
          <Label htmlFor={pathId} className="text-[11px] text-muted-foreground">
            Host folder
          </Label>
          <DirectoryPicker
            value={value.host_path}
            disabled={disabled}
            ariaLabel={`Host folder for share ${index + 1}`}
            onChange={(host_path) => onChange({ ...value, host_path })}
          />
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          disabled={disabled}
          aria-label={`Remove share ${index + 1}`}
          onClick={onRemove}
          className="mt-5 shrink-0 text-muted-foreground"
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <div className="flex flex-col gap-1">
        <Label htmlFor={tagId} className="text-[11px] text-muted-foreground">
          Mount tag
        </Label>
        <Input
          id={tagId}
          spellCheck={false}
          autoComplete="off"
          placeholder="shared"
          aria-label={`Mount tag for share ${index + 1}`}
          aria-invalid={invalid}
          disabled={disabled}
          value={value.mount_tag}
          onChange={(e) => onChange({ ...value, mount_tag: e.target.value })}
          className={cn("font-mono", invalid && "border-destructive")}
        />
      </div>

      <label className="flex items-center gap-2 text-xs text-muted-foreground">
        <input
          type="checkbox"
          className="size-3.5 accent-primary"
          disabled={disabled}
          checked={value.read_only}
          aria-label={`Make share ${index + 1} read-only`}
          onChange={(e) => onChange({ ...value, read_only: e.target.checked })}
        />
        Read-only (guest cannot write to this folder)
      </label>

      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  );
}
