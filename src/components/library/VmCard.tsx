import { Disc, Cpu, MemoryStick, MicrochipIcon } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { StatusBadge } from "@/components/common/StatusBadge";
import { AccelBadge } from "@/components/common/AccelBadge";
import { QuickActions, type VmActions } from "@/components/library/QuickActions";
import { ARCH_LABELS } from "@/components/wizard/steps/StepBasics";
import { basename, formatCpus, formatMemory } from "@/lib/format";
import type { VmListItem } from "@/lib/ipc";

/** A single VM tile: name, status, accel, hardware summary, state-aware actions. */
export function VmCard({
  vm,
  actions,
  busy,
}: {
  vm: VmListItem;
  actions: VmActions;
  busy?: boolean;
}) {
  return (
    <Card className="gap-4 py-4">
      <CardHeader className="px-4">
        <CardTitle className="truncate text-base" title={vm.name}>
          {vm.name}
        </CardTitle>
        <div className="flex flex-wrap items-center gap-2">
          <StatusBadge state={vm.state} suspended={vm.suspended} />
          <AccelBadge accel={vm.accelerator} emulated={vm.emulated} />
        </div>
      </CardHeader>

      <CardContent className="flex flex-col gap-3 px-4">
        <dl className="flex flex-wrap gap-x-5 gap-y-1.5 text-xs text-muted-foreground">
          <div className="flex items-center gap-1.5">
            <Cpu className="h-3.5 w-3.5" /> {formatCpus(vm.cpus)}
          </div>
          <div className="flex items-center gap-1.5">
            <MemoryStick className="h-3.5 w-3.5" /> {formatMemory(vm.memory_mib)}
          </div>
          {vm.guest_arch && (
            <div className="flex items-center gap-1.5">
              <MicrochipIcon className="h-3.5 w-3.5" />{" "}
              {ARCH_LABELS[vm.guest_arch] ?? vm.guest_arch}
            </div>
          )}
          {vm.iso && (
            <div
              className="flex min-w-0 items-center gap-1.5"
              title={vm.iso}
            >
              <Disc className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{basename(vm.iso)}</span>
            </div>
          )}
        </dl>

        <div className="pt-1">
          <QuickActions vm={vm} actions={actions} busy={busy} />
        </div>
      </CardContent>
    </Card>
  );
}
