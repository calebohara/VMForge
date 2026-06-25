import { ArrowDownToLine, ChevronRight, MonitorCog, MoreVertical } from "lucide-react";
import { cn } from "@/lib/utils";
import { AccelBadge } from "@/components/common/AccelBadge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { checkForUpdates } from "@/lib/updater";
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

          {/*
            App-wide overflow menu. Currently just the (inert until activated,
            spec §D / D5) "Check for updates…" affordance bound to
            checkForUpdates(); lives here so it's reachable from every view,
            including the first-run QEMU gate.
          */}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button size="icon-sm" variant="ghost" aria-label="More options">
                <MoreVertical className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onSelect={() => void checkForUpdates()}>
                <ArrowDownToLine className="h-4 w-4" /> Check for updates…
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </header>

        <div className="flex min-h-0 flex-1 flex-col">{children}</div>
      </div>
      <Toaster position="bottom-right" richColors closeButton />
    </TooltipProvider>
  );
}
