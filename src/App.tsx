import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { AppShell, type Crumb } from "@/components/layout/AppShell";
import { LibraryView } from "@/components/library/LibraryView";
import { ConsoleView } from "@/components/console/ConsoleView";
import { NewVmWizard } from "@/components/wizard/NewVmWizard";
import { HardwareEditorView } from "@/components/editor/HardwareEditorView";
import type { VmActions } from "@/components/library/QuickActions";
import { useHostCaps } from "@/hooks/useHostCaps";
import { useVmLibrary } from "@/hooks/useVmLibrary";
import * as ipc from "@/lib/ipc";
import type { VmListItem, VmState } from "@/lib/ipc";

// View state machine (spec §D.5).
type View =
  | { kind: "library" }
  | { kind: "wizard" }
  | { kind: "editor"; vmId: string }
  | { kind: "console"; vmId: string; wsPort: number };

function App() {
  const { caps, hostCores } = useHostCaps();
  const { vms, loading, refresh } = useVmLibrary();

  const [view, setView] = useState<View>({ kind: "library" });
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set());

  const setBusy = useCallback((id: string, busy: boolean) => {
    setBusyIds((prev) => {
      const next = new Set(prev);
      if (busy) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);

  /** Run an action against a VM with a busy marker, toast-on-error, and refresh. */
  const runAction = useCallback(
    async (id: string, label: string, fn: () => Promise<unknown>) => {
      setBusy(id, true);
      try {
        await fn();
        await refresh();
      } catch (e) {
        toast.error(`${label} failed`, { description: String(e) });
      } finally {
        setBusy(id, false);
      }
    },
    [setBusy, refresh],
  );

  const openConsole = useCallback(
    async (vm: VmListItem) => {
      setBusy(vm.id, true);
      try {
        const wsPort = await ipc.openConsole(vm.id);
        setView({ kind: "console", vmId: vm.id, wsPort });
      } catch (e) {
        toast.error("Could not open console", { description: String(e) });
      } finally {
        setBusy(vm.id, false);
      }
    },
    [setBusy],
  );

  const startAndConsole = useCallback(
    async (id: string) => {
      setBusy(id, true);
      try {
        await ipc.startVm(id);
        const wsPort = await ipc.openConsole(id);
        setView({ kind: "console", vmId: id, wsPort });
        void refresh();
      } catch (e) {
        toast.error("Could not start VM", { description: String(e) });
        void refresh();
      } finally {
        setBusy(id, false);
      }
    },
    [setBusy, refresh],
  );

  const actions: VmActions = {
    onStart: (vm) => void startAndConsole(vm.id),
    onShutdown: (vm) =>
      void runAction(vm.id, "Shut down", () => ipc.powerOff(vm.id)),
    onForceOff: (vm) =>
      void runAction(vm.id, "Force off", () => ipc.forceOff(vm.id)),
    onPause: (vm) => void runAction(vm.id, "Pause", () => ipc.pauseVm(vm.id)),
    onResume: (vm) => void runAction(vm.id, "Resume", () => ipc.resumeVm(vm.id)),
    onOpenConsole: (vm) => void openConsole(vm),
    onEdit: (vm) => setView({ kind: "editor", vmId: vm.id }),
    onDelete: () => {
      /* handled by LibraryView's confirmation dialog */
    },
  };

  const confirmDelete = useCallback(
    (id: string, deleteDisks: boolean) =>
      void runAction(id, "Delete", () => ipc.deleteVm(id, deleteDisks)),
    [runAction],
  );

  const backToLibrary = useCallback(() => {
    setView({ kind: "library" });
    void refresh();
  }, [refresh]);

  // ---- Console lifecycle polling (keeps the console toolbar state honest) ----
  const consoleVmId = view.kind === "console" ? view.vmId : null;
  const [consoleState, setConsoleState] = useState<VmState>("running");
  const consoleBusy = consoleVmId ? busyIds.has(consoleVmId) : false;
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (!consoleVmId) return;
    const tick = () =>
      ipc
        .vmState(consoleVmId)
        .then(setConsoleState)
        .catch(() => {
          /* VM may have been removed; toolbar will fall back */
        });
    tick();
    pollRef.current = setInterval(tick, 1500);
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [consoleVmId]);

  // ---- Render ----
  const consoleVm =
    view.kind === "console" ? vms.find((v) => v.id === view.vmId) : undefined;
  const consoleName = consoleVm?.name ?? "Console";

  const breadcrumbs: Crumb[] = (() => {
    switch (view.kind) {
      case "wizard":
        return [
          { label: "Library", onClick: backToLibrary },
          { label: "New VM" },
        ];
      case "editor":
        return [
          { label: "Library", onClick: backToLibrary },
          { label: "Edit" },
        ];
      case "console":
        return [
          { label: "Library", onClick: backToLibrary },
          { label: consoleName },
        ];
      default:
        return [];
    }
  })();

  return (
    <AppShell caps={caps} breadcrumbs={breadcrumbs}>
      {view.kind === "library" && (
        <LibraryView
          vms={vms}
          loading={loading}
          caps={caps}
          busyIds={busyIds}
          actions={actions}
          onCreate={() => setView({ kind: "wizard" })}
          onConfirmDelete={confirmDelete}
        />
      )}

      {view.kind === "wizard" && (
        <NewVmWizard
          hostCores={hostCores}
          onCancel={backToLibrary}
          onCreated={(config, start) => {
            // Leave the wizard immediately so a later start/console failure
            // can't strand the user on a stale wizard. The created VM shows in
            // the library; startAndConsole switches to the console on success
            // or toasts and stays on the library on failure.
            setView({ kind: "library" });
            void refresh();
            if (start) void startAndConsole(config.id);
          }}
        />
      )}

      {view.kind === "editor" && (
        <HardwareEditorView
          vmId={view.vmId}
          state={vms.find((v) => v.id === view.vmId)?.state ?? "stopped"}
          hostCores={hostCores}
          onClose={backToLibrary}
          onSaved={backToLibrary}
        />
      )}

      {view.kind === "console" && (
        <ConsoleView
          name={consoleName}
          wsPort={view.wsPort}
          state={consoleState}
          busy={consoleBusy}
          onPause={() =>
            void runAction(view.vmId, "Pause", () => ipc.pauseVm(view.vmId))
          }
          onResume={() =>
            void runAction(view.vmId, "Resume", () => ipc.resumeVm(view.vmId))
          }
          onShutdown={() =>
            void runAction(view.vmId, "Shut down", () => ipc.powerOff(view.vmId))
          }
          onForceOff={() =>
            runAction(view.vmId, "Force off", () =>
              ipc.forceOff(view.vmId),
            ).then(backToLibrary)
          }
          onBack={backToLibrary}
        />
      )}
    </AppShell>
  );
}

export default App;
