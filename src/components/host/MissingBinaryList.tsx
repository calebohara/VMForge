import { CheckCircle2, XCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import { requiredBinaries } from "@/lib/hostStatus";
import type { HostCapabilities } from "@/lib/ipc";

/**
 * The required-QEMU-binaries checklist for the first-run gate. Each row names a
 * binary and shows whether the probe found it (with its version when present),
 * so the copy is honest about exactly what is missing vs. already installed.
 */
export function MissingBinaryList({ caps }: { caps: HostCapabilities }) {
  const binaries = requiredBinaries(caps);
  return (
    <ul className="space-y-2" aria-label="Required QEMU binaries">
      {binaries.map((b) => (
        <li
          key={b.name}
          className={cn(
            "flex items-center justify-between gap-3 rounded-md border px-3 py-2 text-sm",
            b.present
              ? "border-emerald-500/30 bg-emerald-500/5"
              : "border-destructive/40 bg-destructive/5",
          )}
        >
          <span className="flex items-center gap-2 font-mono text-xs sm:text-sm">
            {b.present ? (
              <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-500" />
            ) : (
              <XCircle className="h-4 w-4 shrink-0 text-destructive" />
            )}
            {b.name}
          </span>
          <span className="text-xs text-muted-foreground">
            {b.present ? (b.version ?? "found") : "not found"}
          </span>
        </li>
      ))}
    </ul>
  );
}
