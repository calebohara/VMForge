import { useMemo } from "react";
import { AlertTriangle, Camera, MemoryStick } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { snapshotDateLabel } from "@/lib/format";
import { buildTree, type TreeNode } from "@/components/snapshots/snapshotTree";
import type { Snapshot } from "@/lib/ipc";

/**
 * Renders the snapshot forest built from the flat DTO list. Degrades to a flat
 * single-level list when every snapshot is top-level (spec §A1). Selection is
 * controlled by the parent {@link SnapshotsView}. Lives alongside
 * {@link SnapshotNode} (a separate file would case-collide with `snapshotTree.ts`
 * on case-insensitive filesystems).
 */
export function SnapshotTree({
  snapshots,
  selectedId,
  disabled,
  onSelect,
}: {
  snapshots: Snapshot[];
  selectedId: string | null;
  disabled?: boolean;
  onSelect: (id: string) => void;
}) {
  const roots = useMemo(() => buildTree(snapshots), [snapshots]);

  return (
    <ul className="space-y-0.5 py-1">
      {roots.map((root) => (
        <SnapshotNode
          key={root.snapshot.snapshot_id}
          node={root}
          selectedId={selectedId}
          disabled={disabled}
          onSelect={onSelect}
        />
      ))}
    </ul>
  );
}

/**
 * A single row in the snapshot tree. Shows the name, created-at, a RAM badge
 * when the snapshot captured memory (`has_vm_state`), and a warning chip when
 * our metadata references a snapshot that's missing from the qcow2
 * (`present_in_qcow2 === false`). Children are rendered by {@link SnapshotTree}
 * via recursion; indentation is driven by `node.depth`.
 */
export function SnapshotNode({
  node,
  selectedId,
  disabled,
  onSelect,
}: {
  node: TreeNode;
  selectedId: string | null;
  disabled?: boolean;
  onSelect: (id: string) => void;
}) {
  const s = node.snapshot;
  const selected = selectedId === s.snapshot_id;

  return (
    <li>
      <button
        type="button"
        disabled={disabled}
        onClick={() => onSelect(s.snapshot_id)}
        style={{ paddingLeft: `${node.depth * 16 + 12}px` }}
        className={cn(
          "flex w-full items-center gap-2 rounded-md py-1.5 pr-3 text-left text-sm transition-colors",
          "hover:bg-accent disabled:pointer-events-none disabled:opacity-60",
          selected && "bg-accent",
        )}
      >
        <Camera className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate" title={s.name}>
          {s.name}
        </span>
        {s.has_vm_state && (
          <Badge variant="secondary" className="gap-1">
            <MemoryStick className="h-3 w-3" /> RAM
          </Badge>
        )}
        {!s.present_in_qcow2 && (
          <Badge
            variant="outline"
            className="gap-1 border-amber-500/40 text-amber-600 dark:text-amber-500"
            title="This snapshot is in your library metadata but missing from the disk image."
          >
            <AlertTriangle className="h-3 w-3" /> Missing
          </Badge>
        )}
        <span className="shrink-0 text-xs text-muted-foreground">
          {snapshotDateLabel(s.created_at)}
        </span>
      </button>

      {node.children.length > 0 && (
        <ul>
          {node.children.map((child) => (
            <SnapshotNode
              key={child.snapshot.snapshot_id}
              node={child}
              selectedId={selectedId}
              disabled={disabled}
              onSelect={onSelect}
            />
          ))}
        </ul>
      )}
    </li>
  );
}
