import {
  Camera,
  Copy,
  Loader2,
  MoreHorizontal,
  Monitor,
  Pause,
  Pencil,
  Play,
  Power,
  Square,
  Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { isLive, isTransitioning } from "@/lib/format";
import type { VmListItem } from "@/lib/ipc";

export interface VmActions {
  onStart: (vm: VmListItem) => void;
  onShutdown: (vm: VmListItem) => void;
  onForceOff: (vm: VmListItem) => void;
  onPause: (vm: VmListItem) => void;
  onResume: (vm: VmListItem) => void;
  onOpenConsole: (vm: VmListItem) => void;
  onEdit: (vm: VmListItem) => void;
  onDelete: (vm: VmListItem) => void;
  /** Open the snapshot manager for this VM (Phase 3). */
  onOpenSnapshots: (vm: VmListItem) => void;
  /** Open the clone dialog with this VM as source (Phase 3). */
  onClone: (vm: VmListItem) => void;
}

/**
 * State-aware action row for a VM:
 *   defined/stopped -> Start, Edit, ⋯
 *   running         -> Open console, Shutdown, Force off, Pause, ⋯
 *   paused          -> Resume, Open console, Force off, ⋯
 *   starting/stopping -> disabled + spinner
 *   error           -> Force off, ⋯
 * The ⋯ overflow menu exposes Snapshots / Clone… / Rename… / Delete. Clone is
 * disabled while the source is live (a stopped source is required, decision A4).
 */
export function QuickActions({
  vm,
  actions,
  busy,
  size = "sm",
}: {
  vm: VmListItem;
  actions: VmActions;
  busy?: boolean;
  size?: "sm" | "default";
}) {
  const transitioning = isTransitioning(vm.state);
  const live = isLive(vm.state);
  const disabled = busy || transitioning;

  if (transitioning) {
    return (
      <Button size={size} variant="outline" disabled>
        <Loader2 className="h-4 w-4 animate-spin" />
        {vm.state === "starting" ? "Starting…" : "Stopping…"}
      </Button>
    );
  }

  const overflow = (
    <OverflowMenu vm={vm} actions={actions} live={live} disabled={disabled} size={size} />
  );

  switch (vm.state) {
    case "defined":
    case "stopped":
      return (
        <div className="flex items-center gap-2">
          <Button size={size} disabled={disabled} onClick={() => actions.onStart(vm)}>
            <Play className="h-4 w-4" /> Start
          </Button>
          <EditButton vm={vm} actions={actions} live={live} disabled={disabled} size={size} />
          {overflow}
        </div>
      );

    case "running":
      return (
        <div className="flex items-center gap-2">
          <Button size={size} disabled={disabled} onClick={() => actions.onOpenConsole(vm)}>
            <Monitor className="h-4 w-4" /> Open console
          </Button>
          <Button
            size={size}
            variant="outline"
            disabled={disabled}
            onClick={() => actions.onPause(vm)}
          >
            <Pause className="h-4 w-4" /> Pause
          </Button>
          <Button
            size={size}
            variant="outline"
            disabled={disabled}
            onClick={() => actions.onShutdown(vm)}
          >
            <Power className="h-4 w-4" /> Shut down
          </Button>
          <Button
            size={size}
            variant="outline"
            disabled={disabled}
            className="text-destructive hover:text-destructive"
            onClick={() => actions.onForceOff(vm)}
          >
            <Square className="h-4 w-4" /> Force off
          </Button>
          {overflow}
        </div>
      );

    case "paused":
      return (
        <div className="flex items-center gap-2">
          <Button size={size} disabled={disabled} onClick={() => actions.onResume(vm)}>
            <Play className="h-4 w-4" /> Resume
          </Button>
          <Button
            size={size}
            variant="outline"
            disabled={disabled}
            onClick={() => actions.onOpenConsole(vm)}
          >
            <Monitor className="h-4 w-4" /> Open console
          </Button>
          <Button
            size={size}
            variant="outline"
            disabled={disabled}
            className="text-destructive hover:text-destructive"
            onClick={() => actions.onForceOff(vm)}
          >
            <Square className="h-4 w-4" /> Force off
          </Button>
          {overflow}
        </div>
      );

    case "error":
      return (
        <div className="flex items-center gap-2">
          <Button
            size={size}
            variant="outline"
            disabled={disabled}
            className="text-destructive hover:text-destructive"
            onClick={() => actions.onForceOff(vm)}
          >
            <Square className="h-4 w-4" /> Force off
          </Button>
          {overflow}
        </div>
      );

    default:
      return null;
  }
}

function OverflowMenu({
  vm,
  actions,
  live,
  disabled,
  size,
}: {
  vm: VmListItem;
  actions: VmActions;
  live: boolean;
  disabled?: boolean;
  size: "sm" | "default";
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          size={size === "sm" ? "icon-sm" : "icon"}
          variant="outline"
          disabled={disabled}
          aria-label="More actions"
        >
          <MoreHorizontal className="h-4 w-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={() => actions.onOpenSnapshots(vm)}>
          <Camera className="h-4 w-4" /> Snapshots
        </DropdownMenuItem>
        <DropdownMenuItem
          disabled={live}
          onSelect={() => actions.onClone(vm)}
          title={live ? "Stop the VM to clone it." : undefined}
        >
          <Copy className="h-4 w-4" /> Clone…
        </DropdownMenuItem>
        <DropdownMenuItem
          disabled={live}
          onSelect={() => actions.onEdit(vm)}
        >
          <Pencil className="h-4 w-4" /> Rename…
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          variant="destructive"
          onSelect={() => actions.onDelete(vm)}
        >
          <Trash2 className="h-4 w-4" /> Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function EditButton({
  vm,
  actions,
  live,
  disabled,
  size,
}: {
  vm: VmListItem;
  actions: VmActions;
  live: boolean;
  disabled?: boolean;
  size: "sm" | "default";
}) {
  const btn = (
    <Button
      size={size}
      variant="outline"
      disabled={disabled || live}
      onClick={() => actions.onEdit(vm)}
    >
      <Pencil className="h-4 w-4" /> Edit
    </Button>
  );
  if (!live) return btn;
  return (
    <Tooltip>
      {/* span wrapper so the tooltip works on a disabled button */}
      <TooltipTrigger asChild>
        <span className="inline-flex">{btn}</span>
      </TooltipTrigger>
      <TooltipContent>Stop the VM to edit its hardware.</TooltipContent>
    </Tooltip>
  );
}
