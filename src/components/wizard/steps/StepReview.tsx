import { Cpu, Disc, HardDrive, MemoryStick, Network, Tag } from "lucide-react";
import { basename, formatCpus, formatMemory } from "@/lib/format";
import type { NetworkMode } from "@/lib/ipc";

const NETWORK_LABELS: Record<NetworkMode, string> = {
  user: "NAT (user mode)",
  bridged: "Bridged",
  "host-only": "Host-only",
};

interface ReviewRow {
  icon: React.ReactNode;
  label: string;
  value: string;
}

/** Step 5 — Review. A read-only summary of every choice before creating. */
export function StepReview({
  name,
  cpus,
  memoryMib,
  diskGib,
  mode,
  iso,
}: {
  name: string;
  cpus: number;
  memoryMib: number;
  diskGib: number;
  mode: NetworkMode;
  iso: string;
}) {
  const rows: ReviewRow[] = [
    { icon: <Tag className="h-4 w-4" />, label: "Name", value: name },
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
      value: NETWORK_LABELS[mode],
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
