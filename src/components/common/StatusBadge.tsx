import { cn } from "@/lib/utils";
import { stateLabel, stateTone, type StateTone } from "@/lib/format";
import type { VmState } from "@/lib/ipc";

const TONE_CLASSES: Record<StateTone, { dot: string; text: string }> = {
  running: { dot: "bg-emerald-500", text: "text-emerald-400" },
  paused: { dot: "bg-amber-500", text: "text-amber-400" },
  idle: { dot: "bg-muted-foreground", text: "text-muted-foreground" },
  transitioning: { dot: "bg-sky-500 animate-pulse", text: "text-sky-400" },
  error: { dot: "bg-destructive", text: "text-destructive" },
};

/** Lifecycle-state pill: colored dot + label. Used in the library and console. */
export function StatusBadge({
  state,
  className,
}: {
  state: VmState;
  className?: string;
}) {
  const tone = TONE_CLASSES[stateTone(state)];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border border-border px-2.5 py-1 text-xs font-medium",
        tone.text,
        className,
      )}
    >
      <span className={cn("h-1.5 w-1.5 rounded-full", tone.dot)} />
      {stateLabel(state)}
    </span>
  );
}
