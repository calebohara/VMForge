import { Pause, Play, Power, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { StatusBadge } from "@/components/common/StatusBadge";
import { VncConsole } from "@/components/VncConsole";
import type { VmState } from "@/lib/ipc";

/**
 * Console view — preserves the Phase-1 console UX (StatusBadge +
 * Pause/Resume/Shutdown/Force off) relocated into the dedicated view, and
 * renders the embedded {@link VncConsole} (console-engineer's component,
 * unchanged).
 */
export function ConsoleView({
  name,
  wsPort,
  state,
  busy,
  onPause,
  onResume,
  onShutdown,
  onForceOff,
  onBack,
}: {
  name: string;
  wsPort: number;
  state: VmState;
  busy?: boolean;
  onPause: () => void;
  onResume: () => void;
  onShutdown: () => void;
  onForceOff: () => void;
  onBack: () => void;
}) {
  const showConsole =
    state === "running" || state === "paused" || state === "starting";

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b border-border px-5 py-2">
        <span className="mr-auto truncate text-sm font-medium" title={name}>
          {name}
        </span>
        <StatusBadge state={state} />
        {state === "paused" ? (
          <Button size="sm" variant="outline" disabled={busy} onClick={onResume}>
            <Play className="h-4 w-4" /> Resume
          </Button>
        ) : (
          <Button
            size="sm"
            variant="outline"
            disabled={busy || state !== "running"}
            onClick={onPause}
          >
            <Pause className="h-4 w-4" /> Pause
          </Button>
        )}
        <Button size="sm" variant="outline" disabled={busy} onClick={onShutdown}>
          <Power className="h-4 w-4" /> Shut down
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          className="text-destructive hover:text-destructive"
          onClick={onForceOff}
        >
          <Square className="h-4 w-4" /> Force off
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-hidden">
        {showConsole ? (
          <VncConsole wsPort={wsPort} />
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-sm text-muted-foreground">
            <p>The VM has stopped.</p>
            <Button variant="outline" onClick={onBack}>
              Back to library
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
