import { useCallback, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderSearch, Loader2, PackageX, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { MissingBinaryList } from "@/components/host/MissingBinaryList";
import { InstallInstructions } from "@/components/host/InstallInstructions";
import { setQemuDir, type HostCapabilities } from "@/lib/ipc";

export interface QemuRequiredGateProps {
  caps: HostCapabilities;
  /** The last probe error, if the probe itself failed (vs. QEMU just missing). */
  error?: string | null;
  /** True while a Re-check / Locate re-probe is in flight (drives spinners). */
  rechecking?: boolean;
  /**
   * Re-probe the host (no app restart). Resolves with fresh caps; the parent
   * owns the probe and will fall through to the library once QEMU resolves.
   */
  onRecheck: () => Promise<HostCapabilities | null>;
}

/**
 * The hard first-run gate (spec §C): shown via App.tsx early-return when
 * `qemuMissing(caps)`. Names the actual missing binaries (and versions already
 * found), gives honest per-OS install instructions, and offers two ways
 * forward without restarting the app:
 *
 *  - **Re-check** — re-invoke `probe_host` after the user installs QEMU.
 *  - **Locate QEMU…** — pick the directory holding the QEMU binaries; this
 *    persists the D3 override (`set_qemu_dir`) that `resolve_qemu_binary` reads,
 *    then re-probes. This is the escape hatch for the macOS empty-PATH-under-
 *    Finder case where QEMU is installed but unreachable on `$PATH`.
 */
export function QemuRequiredGate({
  caps,
  error,
  rechecking = false,
  onRecheck,
}: QemuRequiredGateProps) {
  // Local busy flag for the Locate flow (pick + persist), independent of the
  // parent's `rechecking` (which covers the probe). Both disable the buttons.
  const [locating, setLocating] = useState(false);
  const busy = rechecking || locating;

  const locate = useCallback(async () => {
    setLocating(true);
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected !== "string") return; // user cancelled
      await setQemuDir(selected);
      await onRecheck();
    } catch {
      // setQemuDir / picker failure is non-fatal; the gate stays up and the
      // user can retry. (A toast would need the AppShell Toaster, which wraps
      // this component, but we keep the gate self-contained + silent here.)
    } finally {
      setLocating(false);
    }
  }, [onRecheck]);

  return (
    <div className="flex min-h-0 flex-1 items-start justify-center overflow-auto p-6">
      <Card className="w-full max-w-2xl">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <PackageX className="h-5 w-5 text-destructive" />
            QEMU is required to run VMForge
          </CardTitle>
        </CardHeader>

        <CardContent className="space-y-5">
          <p className="text-sm text-muted-foreground">
            VMForge drives QEMU to run virtual machines — it does not bundle it.
            Install QEMU (or point VMForge at an existing install) and re-check;
            no app restart is needed.
          </p>

          {error && (
            <p
              className="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive"
              role="alert"
            >
              Probe error: {error}
            </p>
          )}

          <MissingBinaryList caps={caps} />

          <Separator />

          <InstallInstructions os={caps.os} />

          <Separator />

          <div className="flex flex-wrap gap-2">
            <Button onClick={() => void onRecheck()} disabled={busy}>
              {rechecking ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <RefreshCw className="h-4 w-4" />
              )}
              Re-check
            </Button>
            <Button
              variant="outline"
              onClick={() => void locate()}
              disabled={busy}
            >
              {locating ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <FolderSearch className="h-4 w-4" />
              )}
              Locate QEMU…
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
