import { Cpu, Zap } from "lucide-react";
import { cn } from "@/lib/utils";
import { accelLabel, isHardwareAccel } from "@/lib/format";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { Accelerator } from "@/lib/ipc";

/**
 * Accelerator pill. Hardware-accelerated backends are emerald; TCG (software
 * emulation) is amber, with an honest performance-warning tooltip.
 */
export function AccelBadge({
  accel,
  emulated = false,
  className,
}: {
  accel: Accelerator;
  emulated?: boolean;
  className?: string;
}) {
  const hw = isHardwareAccel(accel);
  const label = accelLabel(accel);

  const badge = (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full px-2.5 py-1 text-xs font-medium",
        hw
          ? "bg-emerald-500/15 text-emerald-400"
          : "bg-amber-500/15 text-amber-400",
        className,
      )}
    >
      {hw ? <Zap className="h-3 w-3" /> : <Cpu className="h-3 w-3" />}
      {label}
      {!hw && " · software"}
      {emulated && " · emulated"}
    </span>
  );

  if (hw && !emulated) return badge;

  return (
    <Tooltip>
      <TooltipTrigger asChild>{badge}</TooltipTrigger>
      <TooltipContent>
        {!hw
          ? "Software emulation (TCG) — expect significantly reduced performance."
          : "Emulated guest architecture — expect reduced performance."}
      </TooltipContent>
    </Tooltip>
  );
}
