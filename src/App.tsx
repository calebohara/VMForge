import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { AppShell, type Crumb } from "@/components/layout/AppShell";
import { LibraryView } from "@/components/library/LibraryView";
import { ConsoleView } from "@/components/console/ConsoleView";
import { NewVmWizard } from "@/components/wizard/NewVmWizard";
import { HardwareEditorView } from "@/components/editor/HardwareEditorView";
import { SnapshotsView } from "@/components/snapshots/SnapshotsView";
import { HostProbeLoading } from "@/components/host/HostProbeLoading";
import { QemuRequiredGate } from "@/components/host/QemuRequiredGate";
import type { VmActions } from "@/components/library/QuickActions";
import { useHostCaps } from "@/hooks/useHostCaps";
import { useVmLibrary } from "@/hooks/useVmLibrary";
import { qemuMissing } from "@/lib/hostStatus";
import * as ipc from "@/lib/ipc";
import type { VmListItem, VmState } from "@/lib/ipc";

// View state machine (spec §D.5).
type View =
  | { kind: "library" }
  | { kind: "wizard" }
  | { kind: "editor"; vmId: string }
  | { kind: "snapshots"; vmId: string }
  | { kind: "console"; vmId: string; wsPort: number };

function App() {
  const {
    caps,
    hostCores,
    loading: capsLoading,
    refreshing: capsRefreshing,
    error: capsError,
    refresh: refreshCaps,
  } = useHostCaps();
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
        return true;
      } catch (e) {
        toast.error(`${label} failed`, { description: String(e) });
        return false;
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

  // Restore a suspended VM and jump straight into its console — mirrors
  // startAndConsole, but uses restore_vm (relaunch + snapshot-load + cont).
  const restoreAndConsole = useCallback(
    async (id: string) => {
      setBusy(id, true);
      try {
        await ipc.restoreVm(id);
        const wsPort = await ipc.openConsole(id);
        setView({ kind: "console", vmId: id, wsPort });
        void refresh();
      } catch (e) {
        toast.error("Could not resume VM", { description: String(e) });
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
    onOpenSnapshots: (vm) => setView({ kind: "snapshots", vmId: vm.id }),
    onClone: () => {
      /* handled by LibraryView's clone dialog */
    },
    onSuspend: (vm) =>
      void runAction(vm.id, "Suspend", () => ipc.suspendVm(vm.id)),
    onRestore: (vm) => void restoreAndConsole(vm.id),
    onDiscard: (vm) =>
      void runAction(vm.id, "Discard suspend", () =>
        ipc.discardSuspend(vm.id),
      ),
  };

  const confirmDelete = useCallback(
    (id: string, deleteDisks: boolean) =>
      void runAction(id, "Delete", () => ipc.deleteVm(id, deleteDisks)),
    [runAction],
  );

  // Clone resolves on completion so the dialog holds its "Cloning…" spinner for
  // the whole synchronous op; on success we refresh so the new VM appears.
  const confirmClone = useCallback(
    async (id: string, newName: string, linked: boolean) => {
      setBusy(id, true);
      try {
        const clone = await ipc.cloneVm(id, newName, linked);
        await refresh();
        toast.success(`Created “${clone.name}”`);
      } catch (e) {
        toast.error("Clone failed", { description: String(e) });
        throw e;
      } finally {
        setBusy(id, false);
      }
    },
    [setBusy, refresh],
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
      case "snapshots": {
        const snapVm = vms.find((v) => v.id === view.vmId);
        return [
          { label: "Library", onClick: backToLibrary },
          { label: snapVm?.name ?? "VM" },
          { label: "Snapshots" },
        ];
      }
      case "console":
        return [
          { label: "Library", onClick: backToLibrary },
          { label: consoleName },
        ];
      default:
        return [];
    }
  })();

  // ---- First-run host gate (spec §C) ----
  // Early-return BEFORE the view machine. The initial probe shows a loading
  // screen; a host with no usable QEMU shows the hard gate inside AppShell
  // (Re-check re-probes with no restart, falling through to the views once
  // QEMU resolves). Otherwise the existing views render unchanged.
  if (caps === null && capsLoading) {
    return <HostProbeLoading />;
  }
  if (qemuMissing(caps)) {
    return (
      <AppShell caps={caps}>
        <QemuRequiredGate
          caps={caps!}
          error={capsError}
          rechecking={capsRefreshing}
          onRecheck={refreshCaps}
        />
      </AppShell>
    );
  }

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
          onConfirmClone={confirmClone}
        />
      )}

      {view.kind === "wizard" && (
        <NewVmWizard
          hostCores={hostCores}
          hostArch={caps?.arch ?? null}
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
          onOpenSnapshots={() =>
            setView({ kind: "snapshots", vmId: view.vmId })
          }
        />
      )}

      {view.kind === "snapshots" && (
        <SnapshotsView
          vmId={view.vmId}
          vmName={vms.find((v) => v.id === view.vmId)?.name ?? "VM"}
          state={vms.find((v) => v.id === view.vmId)?.state ?? "stopped"}
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
          onSuspend={() =>
            void runAction(view.vmId, "Suspend", () =>
              ipc.suspendVm(view.vmId),
            ).then((ok) => {
              // Only leave the console if the suspend actually succeeded; on a
              // refusal (e.g. HVF gate) keep the still-running console up.
              if (ok) backToLibrary();
            })
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
          onOpenSnapshots={() =>
            setView({ kind: "snapshots", vmId: view.vmId })
          }
        />
      )}
    </AppShell>
  );
}

export default App;
