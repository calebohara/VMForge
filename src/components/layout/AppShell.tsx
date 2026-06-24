import { ChevronRight, MonitorCog } from "lucide-react";
import { cn } from "@/lib/utils";
import { AccelBadge } from "@/components/common/AccelBadge";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { HostCapabilities } from "@/lib/ipc";

export interface Crumb {
  label: string;
  onClick?: () => void;
}

/**
 * Application chrome: titlebar (logo + breadcrumb + AccelBadge), an optional
 * toolbar slot on the right, the body, and the sonner Toaster. Wraps everything
 * in a TooltipProvider so common-component tooltips work app-wide.
 */
export function AppShell({
  caps,
  breadcrumbs = [],
  toolbar,
  children,
}: {
  caps: HostCapabilities | null;
  breadcrumbs?: Crumb[];
  toolbar?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <TooltipProvider>
      <div className="flex h-full flex-col bg-background text-foreground">
        <header className="flex items-center gap-3 border-b border-border px-5 py-3">
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-primary text-primary-foreground">
            <MonitorCog className="h-5 w-5" />
          </div>

          <nav className="mr-auto flex items-center gap-1.5 text-sm">
            <span className="font-semibold leading-tight">VMForge</span>
            {breadcrumbs.map((c, i) => (
              <span key={i} className="flex items-center gap-1.5">
                <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
                {c.onClick ? (
                  <button
                    onClick={c.onClick}
                    className={cn(
                      "text-muted-foreground transition-colors hover:text-foreground",
                      i === breadcrumbs.length - 1 && "text-foreground",
                    )}
                  >
                    {c.label}
                  </button>
                ) : (
                  <span
                    className={cn(
                      "text-muted-foreground",
                      i === breadcrumbs.length - 1 && "text-foreground",
                    )}
                  >
                    {c.label}
                  </span>
                )}
              </span>
            ))}
          </nav>

          {toolbar}

          {caps && (
            <AccelBadge accel={caps.preferred_accelerator} />
          )}
        </header>

        <div className="flex min-h-0 flex-1 flex-col">{children}</div>
      </div>
      <Toaster position="bottom-right" richColors closeButton />
    </TooltipProvider>
  );
}
