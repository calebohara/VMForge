import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  FolderOpen,
  MonitorCog,
  Pause,
  Play,
  Power,
  Rocket,
  Square,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { VncConsole } from "@/components/VncConsole";
import * as ipc from "@/lib/ipc";
import type { HostCapabilities, VmDescriptor, VmState } from "@/lib/ipc";

type View = "setup" | "console";

function App() {
  const [caps, setCaps] = useState<HostCapabilities | null>(null);
  const [view, setView] = useState<View>("setup");
  const [error, setError] = useState<string | null>(null);

  // New-VM form.
  const [name, setName] = useState("alpine-vm");
  const [cpus, setCpus] = useState(2);
  const [memMib, setMemMib] = useState(2048);
  const [diskGib, setDiskGib] = useState(8);
  const [iso, setIso] = useState("");
  const [launching, setLaunching] = useState(false);

  // Running VM.
  const [vm, setVm] = useState<VmDescriptor | null>(null);
  const [wsPort, setWsPort] = useState<number | null>(null);
  const [state, setState] = useState<VmState | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    ipc.probeHost().then(setCaps).catch((e) => setError(String(e)));
  }, []);

  // Poll lifecycle state while a console is open.
  const pollRef = useRef<number | null>(null);
  useEffect(() => {
    if (view !== "console" || !vm) return;
    const tick = () =>
      ipc.vmState(vm.id).then(setState).catch(() => {
        /* VM may have been removed */
      });
    tick();
    pollRef.current = window.setInterval(tick, 1500);
    return () => {
      if (pollRef.current) window.clearInterval(pollRef.current);
    };
  }, [view, vm]);

  const browseIso = useCallback(async () => {
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Disc image", extensions: ["iso", "img"] }],
    });
    if (typeof selected === "string") setIso(selected);
  }, []);

  const launch = useCallback(async () => {
    if (!iso) {
      setError("Choose an ISO to boot first.");
      return;
    }
    setError(null);
    setLaunching(true);
    try {
      const desc = await ipc.createAndStartVm({
        name,
        cpus,
        memory_mib: memMib,
        disk_gib: diskGib,
        iso,
      });
      setVm(desc);
      const port = await ipc.openConsole(desc.id);
      setWsPort(port);
      setState("running");
      setView("console");
    } catch (e) {
      setError(String(e));
    } finally {
      setLaunching(false);
    }
  }, [name, cpus, memMib, diskGib, iso]);

  const doAction = useCallback(
    async (fn: (id: string) => Promise<void>) => {
      if (!vm) return;
      setBusy(true);
      try {
        await fn(vm.id);
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    },
    [vm],
  );

  const backToSetup = useCallback(() => {
    setView("setup");
    setVm(null);
    setWsPort(null);
    setState(null);
  }, []);

  const accel = caps?.preferred_accelerator;

  return (
    <div className="flex h-full flex-col bg-background text-foreground">
      {/* Title bar */}
      <header className="flex items-center gap-3 border-b border-border px-5 py-3">
        <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-primary text-primary-foreground">
          <MonitorCog className="h-5 w-5" />
        </div>
        <div className="mr-auto">
          <h1 className="text-base font-semibold leading-tight">VMForge</h1>
          <p className="text-[11px] text-muted-foreground">QEMU-powered VM manager</p>
        </div>

        {accel && (
          <span
            className={cn(
              "rounded-full px-2.5 py-1 text-xs font-medium",
              caps?.hardware_accelerated
                ? "bg-emerald-500/15 text-emerald-400"
                : "bg-amber-500/15 text-amber-400",
            )}
          >
            {accel.toUpperCase()}
            {caps?.hardware_accelerated ? "" : " (software)"}
          </span>
        )}

        {view === "console" && vm && (
          <div className="flex items-center gap-2">
            <StateBadge state={state} />
            {state === "paused" ? (
              <ToolbarButton onClick={() => doAction(ipc.resumeVm)} disabled={busy}>
                <Play className="h-4 w-4" /> Resume
              </ToolbarButton>
            ) : (
              <ToolbarButton onClick={() => doAction(ipc.pauseVm)} disabled={busy}>
                <Pause className="h-4 w-4" /> Pause
              </ToolbarButton>
            )}
            <ToolbarButton onClick={() => doAction(ipc.powerOff)} disabled={busy}>
              <Power className="h-4 w-4" /> Shut down
            </ToolbarButton>
            <ToolbarButton
              onClick={() => doAction(ipc.forceOff).then(backToSetup)}
              disabled={busy}
              variant="danger"
            >
              <Square className="h-4 w-4" /> Force off
            </ToolbarButton>
          </div>
        )}
      </header>

      {error && (
        <div className="flex items-start gap-2 border-b border-destructive/30 bg-destructive/10 px-5 py-2 text-sm">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
          <span className="flex-1">{error}</span>
          <button className="text-xs text-muted-foreground" onClick={() => setError(null)}>
            dismiss
          </button>
        </div>
      )}

      {/* Body */}
      {view === "setup" ? (
        <main className="flex-1 overflow-auto p-6">
          <div className="mx-auto max-w-md space-y-5 rounded-xl border border-border bg-card p-6">
            <div>
              <h2 className="text-sm font-medium">New virtual machine</h2>
              <p className="text-xs text-muted-foreground">
                Phase 1 slice: boot an ISO, view the console, power off.
              </p>
            </div>

            <Field label="Name">
              <input
                className={inputCls}
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </Field>

            <Field label="Boot ISO">
              <div className="flex gap-2">
                <input
                  className={cn(inputCls, "flex-1")}
                  value={iso}
                  placeholder="Choose an .iso…"
                  onChange={(e) => setIso(e.target.value)}
                />
                <button className={btnSecondary} onClick={browseIso} type="button">
                  <FolderOpen className="h-4 w-4" /> Browse
                </button>
              </div>
            </Field>

            <div className="grid grid-cols-3 gap-3">
              <Field label="vCPUs">
                <input
                  type="number"
                  min={1}
                  className={inputCls}
                  value={cpus}
                  onChange={(e) => setCpus(Number(e.target.value) || 1)}
                />
              </Field>
              <Field label="RAM (MiB)">
                <input
                  type="number"
                  min={256}
                  step={256}
                  className={inputCls}
                  value={memMib}
                  onChange={(e) => setMemMib(Number(e.target.value) || 256)}
                />
              </Field>
              <Field label="Disk (GiB)">
                <input
                  type="number"
                  min={1}
                  className={inputCls}
                  value={diskGib}
                  onChange={(e) => setDiskGib(Number(e.target.value) || 1)}
                />
              </Field>
            </div>

            <button
              className={cn(btnPrimary, "w-full justify-center")}
              onClick={launch}
              disabled={launching}
            >
              <Rocket className="h-4 w-4" />
              {launching ? "Launching…" : "Create & start"}
            </button>

            {caps?.warnings?.map((w, i) => (
              <div
                key={i}
                className="flex items-start gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 p-2.5 text-xs"
              >
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-500" />
                <span>{w}</span>
              </div>
            ))}
          </div>
        </main>
      ) : (
        <main className="flex-1 overflow-hidden">
          {wsPort != null && state !== "stopped" ? (
            <VncConsole wsPort={wsPort} />
          ) : (
            <div className="flex h-full flex-col items-center justify-center gap-3 text-sm text-muted-foreground">
              <p>VM stopped.</p>
              <button className={btnSecondary} onClick={backToSetup}>
                Back
              </button>
            </div>
          )}
        </main>
      )}
    </div>
  );
}

const inputCls =
  "rounded-md border border-input bg-background px-3 py-1.5 text-sm outline-none focus:border-ring";
const btnPrimary =
  "inline-flex items-center gap-2 rounded-md bg-primary px-3 py-2 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50";
const btnSecondary =
  "inline-flex items-center gap-2 rounded-md border border-border bg-secondary px-3 py-1.5 text-sm hover:bg-accent";

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}

function ToolbarButton({
  children,
  onClick,
  disabled,
  variant,
}: {
  children: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
  variant?: "danger";
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-xs font-medium disabled:opacity-50",
        variant === "danger"
          ? "border-destructive/40 text-destructive hover:bg-destructive/10"
          : "border-border hover:bg-accent",
      )}
    >
      {children}
    </button>
  );
}

function StateBadge({ state }: { state: VmState | null }) {
  const color =
    state === "running"
      ? "bg-emerald-500"
      : state === "paused"
        ? "bg-amber-500"
        : state === "stopped"
          ? "bg-muted-foreground"
          : "bg-sky-500";
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-border px-2.5 py-1 text-xs">
      <span className={cn("h-1.5 w-1.5 rounded-full", color)} />
      {state ?? "…"}
    </span>
  );
}

export default App;
