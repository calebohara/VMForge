import {
  Cpu,
  Disc,
  Fingerprint,
  HardDrive,
  MemoryStick,
  MicrochipIcon,
  Network,
  Share2,
  Tag,
} from "lucide-react";
import { basename, formatCpus, formatMemory } from "@/lib/format";
import { NETWORK_MODE_LABELS } from "@/components/common/NetworkForm";
import { ARCH_LABELS } from "@/components/wizard/steps/StepBasics";
import type { NetworkConfig } from "@/lib/ipc";

interface ReviewRow {
  icon: React.ReactNode;
  label: string;
  value: string;
}

/** Step 5 — Review. A read-only summary of every choice before creating. */
export function StepReview({
  name,
  arch,
  emulated,
  cpus,
  memoryMib,
  diskGib,
  network,
  iso,
}: {
  name: string;
  arch: string;
  emulated: boolean;
  cpus: number;
  memoryMib: number;
  diskGib: number;
  network: NetworkConfig;
  iso: string;
}) {
  const forwardCount = network.port_forwards.filter(
    (pf) => Number.isInteger(pf.host) && Number.isInteger(pf.guest),
  ).length;
  const forwardLabel =
    network.mode === "user"
      ? forwardCount === 1
        ? "1 forward"
        : `${forwardCount} forwards`
      : "—";

  const rows: ReviewRow[] = [
    { icon: <Tag className="h-4 w-4" />, label: "Name", value: name },
    {
      icon: <MicrochipIcon className="h-4 w-4" />,
      label: "Architecture",
      value: `${ARCH_LABELS[arch] ?? arch}${emulated ? " — emulated (TCG, slow)" : ""}`,
    },
    {
      icon: <Cpu className="h-4 w-4" />,
      label: "Processors",
      value: formatCpus(cpus),
    },
    {
      icon: <MemoryStick className="h-4 w-4" />,
      label: "Memory",
      value: formatMemory(memoryMib),
    },
    {
      icon: <HardDrive className="h-4 w-4" />,
      label: "Disk",
      value: `${diskGib} GiB qcow2`,
    },
    {
      icon: <Network className="h-4 w-4" />,
      label: "Network",
      value: NETWORK_MODE_LABELS[network.mode],
    },
    {
      icon: <Fingerprint className="h-4 w-4" />,
      label: "MAC",
      value: network.mac && network.mac.trim() ? network.mac.trim() : "Auto",
    },
    {
      icon: <Share2 className="h-4 w-4" />,
      label: "Port forwards",
      value: forwardLabel,
    },
    {
      icon: <Disc className="h-4 w-4" />,
      label: "Installer ISO",
      value: iso ? basename(iso) : "None",
    },
  ];

  return (
    <div className="flex flex-col gap-3">
      <dl className="divide-y divide-border overflow-hidden rounded-lg border border-border">
        {rows.map((r) => (
          <div
            key={r.label}
            className="flex items-center gap-3 px-4 py-2.5 text-sm"
          >
            <span className="text-muted-foreground">{r.icon}</span>
            <dt className="w-28 shrink-0 text-muted-foreground">{r.label}</dt>
            <dd
              className="min-w-0 flex-1 truncate text-right font-medium"
              title={r.label === "Installer ISO" && iso ? iso : undefined}
            >
              {r.value}
            </dd>
          </div>
        ))}
      </dl>
      <p className="text-xs text-muted-foreground">
        “Create &amp; start” boots the VM and opens its console. “Create only”
        adds it to your library as stopped.
      </p>
    </div>
  );
}
