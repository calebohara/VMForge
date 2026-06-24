import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  Cpu,
  HardDrive,
  MonitorCog,
  Rocket,
  ShieldCheck,
} from "lucide-react";
import { cn } from "@/lib/utils";

type Accelerator = "hvf" | "whpx" | "kvm" | "tcg";

interface QemuBinary {
  name: string;
  present: boolean;
  version: string | null;
}

interface HostCapabilities {
  os: string;
  arch: string;
  preferred_accelerator: Accelerator;
  available_accelerators: Accelerator[];
  hardware_accelerated: boolean;
  qemu_img: QemuBinary;
  system_binaries: QemuBinary[];
  warnings: string[];
}

const ACCEL_LABEL: Record<Accelerator, string> = {
  hvf: "HVF (Hypervisor.framework)",
  whpx: "WHPX (Windows Hypervisor Platform)",
  kvm: "KVM",
  tcg: "TCG (software emulation)",
};

function App() {
  const [caps, setCaps] = useState<HostCapabilities | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<HostCapabilities>("probe_host")
      .then(setCaps)
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <div className="flex h-full flex-col bg-background text-foreground">
      <header className="flex items-center gap-3 border-b border-border px-6 py-4">
        <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary text-primary-foreground">
          <MonitorCog className="h-5 w-5" />
        </div>
        <div>
          <h1 className="text-lg font-semibold leading-tight">VMForge</h1>
          <p className="text-xs text-muted-foreground">
            A QEMU-powered virtual machine manager
          </p>
        </div>
      </header>

      <main className="flex-1 overflow-auto p-6">
        {!caps && !error && (
          <p className="text-sm text-muted-foreground">Probing host…</p>
        )}

        {error && (
          <div className="flex items-start gap-2 rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm">
            <AlertTriangle className="mt-0.5 h-4 w-4 text-destructive" />
            <span>Host probe failed: {error}</span>
          </div>
        )}

        {caps && (
          <div className="mx-auto max-w-3xl space-y-6">
            <section className="rounded-xl border border-border bg-card p-5">
              <div className="mb-4 flex items-center justify-between">
                <h2 className="text-sm font-medium text-muted-foreground">
                  Host capabilities
                </h2>
                <AccelBadge
                  accel={caps.preferred_accelerator}
                  hardware={caps.hardware_accelerated}
                />
              </div>

              <dl className="grid grid-cols-2 gap-x-8 gap-y-3 text-sm">
                <Row icon={<Cpu className="h-4 w-4" />} label="Platform">
                  {caps.os} · {caps.arch}
                </Row>
                <Row
                  icon={<Rocket className="h-4 w-4" />}
                  label="Preferred accelerator"
                >
                  {ACCEL_LABEL[caps.preferred_accelerator]}
                </Row>
                <Row
                  icon={<ShieldCheck className="h-4 w-4" />}
                  label="Available accelerators"
                >
                  {caps.available_accelerators.length
                    ? caps.available_accelerators
                        .map((a) => a.toUpperCase())
                        .join(", ")
                    : "none detected"}
                </Row>
                <Row
                  icon={<HardDrive className="h-4 w-4" />}
                  label="qemu-img"
                >
                  {caps.qemu_img.present
                    ? `v${caps.qemu_img.version}`
                    : "not found"}
                </Row>
              </dl>

              <div className="mt-4 border-t border-border pt-4">
                <p className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  QEMU system binaries
                </p>
                <ul className="space-y-1 text-sm">
                  {caps.system_binaries.map((b) => (
                    <li key={b.name} className="flex items-center gap-2">
                      <span
                        className={cn(
                          "inline-block h-1.5 w-1.5 rounded-full",
                          b.present ? "bg-emerald-500" : "bg-muted-foreground",
                        )}
                      />
                      <span className="font-mono text-xs">{b.name}</span>
                      <span className="text-muted-foreground">
                        {b.present ? `v${b.version}` : "not installed"}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            </section>

            {caps.warnings.length > 0 && (
              <section className="space-y-2">
                {caps.warnings.map((w, i) => (
                  <div
                    key={i}
                    className="flex items-start gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 p-3 text-sm"
                  >
                    <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-500" />
                    <span>{w}</span>
                  </div>
                ))}
              </section>
            )}

            <p className="text-xs text-muted-foreground">
              Phase 0 complete — the frontend reached the Rust core over IPC and
              the engine probe ran. Next: the Phase 1 vertical slice (ISO →
              launch → console → power off).
            </p>
          </div>
        )}
      </main>
    </div>
  );
}

function Row({
  icon,
  label,
  children,
}: {
  icon: React.ReactNode;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="flex items-center gap-1.5 text-xs text-muted-foreground">
        {icon}
        {label}
      </dt>
      <dd className="font-medium">{children}</dd>
    </div>
  );
}

function AccelBadge({
  accel,
  hardware,
}: {
  accel: Accelerator;
  hardware: boolean;
}) {
  return (
    <span
      className={cn(
        "rounded-full px-2.5 py-1 text-xs font-medium",
        hardware
          ? "bg-emerald-500/15 text-emerald-400"
          : "bg-amber-500/15 text-amber-400",
      )}
    >
      {hardware ? "Hardware accelerated" : "Software (TCG)"} · {accel.toUpperCase()}
    </span>
  );
}

export default App;
