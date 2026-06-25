import { useMemo, useState } from "react";
import { ArrowLeft, ArrowRight, Loader2, Play, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { StepBasics } from "@/components/wizard/steps/StepBasics";
import { StepCpuMemory } from "@/components/wizard/steps/StepCpuMemory";
import { StepStorage } from "@/components/wizard/steps/StepStorage";
import { StepNetwork } from "@/components/wizard/steps/StepNetwork";
import { StepReview } from "@/components/wizard/steps/StepReview";
import {
  normalizeNetwork,
  validateCpus,
  validateDiskGib,
  validateMemoryMib,
  validateVmName,
} from "@/lib/validation";
import { createVm } from "@/lib/ipc";
import type { CreateVmRequest, NetworkConfig, VmConfig } from "@/lib/ipc";

interface WizardDraft {
  name: string;
  iso: string;
  /** Guest arch; "" means "use host arch" until the user picks one. */
  guestArch: string;
  cpus: number;
  memoryMib: number;
  diskGib: number;
  /** Full network config (A10): mode + MAC + port forwards round-trip. */
  network: NetworkConfig;
}

const INITIAL_DRAFT: WizardDraft = {
  name: "",
  iso: "",
  guestArch: "",
  cpus: 2,
  memoryMib: 2048,
  diskGib: 20,
  network: { mode: "user", mac: null, port_forwards: [] },
};

const STEPS = [
  { key: "basics", title: "Basics", subtitle: "Name your VM and pick an ISO" },
  { key: "cpu", title: "CPU & Memory", subtitle: "Allocate processors and RAM" },
  { key: "storage", title: "Storage", subtitle: "Size the primary disk" },
  { key: "network", title: "Network", subtitle: "Choose the adapter mode" },
  { key: "review", title: "Review", subtitle: "Confirm and create" },
] as const;

/**
 * Multi-step New-VM wizard (spec §D). Steps: Basics → CPU/Memory → Storage →
 * Network → Review. Validation gates each step's "Next". On the final step the
 * user chooses "Create & start" (create → start → open console, handled by the
 * parent via {@link onCreated} with `start=true`) or "Create only" (create →
 * back to library).
 */
export function NewVmWizard({
  hostCores,
  hostArch,
  onCreated,
  onCancel,
}: {
  hostCores: number | null;
  /** Host CPU arch from the capability probe (drives the arch default). */
  hostArch: string | null;
  /**
   * Called after a successful `create_vm`. `start` indicates the user chose
   * "Create & start" (the parent should start the VM and open its console).
   */
  onCreated: (config: VmConfig, start: boolean) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState<WizardDraft>(INITIAL_DRAFT);
  const [stepIndex, setStepIndex] = useState(0);
  const [networkValid, setNetworkValid] = useState(true);
  const [submitting, setSubmitting] = useState<null | "start" | "only">(null);
  const [error, setError] = useState<string | null>(null);

  const patch = (p: Partial<WizardDraft>) =>
    setDraft((d) => ({ ...d, ...p }));

  // Per-step validity gates "Next" / "Create".
  const stepValid = useMemo(() => {
    switch (STEPS[stepIndex].key) {
      case "basics":
        return validateVmName(draft.name) === null;
      case "cpu":
        return (
          validateCpus(draft.cpus) === null &&
          validateMemoryMib(draft.memoryMib) === null
        );
      case "storage":
        return validateDiskGib(draft.diskGib) === null;
      case "network":
        return networkValid;
      default:
        return true;
    }
  }, [stepIndex, draft, networkValid]);

  const isLast = stepIndex === STEPS.length - 1;
  const busy = submitting !== null;

  const submit = async (start: boolean) => {
    setSubmitting(start ? "start" : "only");
    setError(null);
    try {
      const req: CreateVmRequest = {
        name: draft.name.trim(),
        hardware: { cpus: draft.cpus, memory_mib: draft.memoryMib },
        disk_gib: draft.diskGib,
        network: normalizeNetwork(draft.network),
        iso: draft.iso.trim() ? draft.iso.trim() : null,
        // Record the explicit guest arch (defaulting to the host's).
        guest_arch: draft.guestArch || hostArch || null,
      };
      const config = await createVm(req);
      onCreated(config, start);
    } catch (e) {
      setError(String(e));
    } finally {
      // Always clear the busy state. On the happy path the parent unmounts the
      // wizard; on failure the footer becomes usable again (no permanent freeze
      // if create succeeds but a later start/console step fails upstream).
      setSubmitting(null);
    }
  };

  const current = STEPS[stepIndex];

  return (
    <div className="mx-auto flex min-h-0 w-full max-w-2xl flex-1 flex-col p-5">
      <Card className="flex min-h-0 flex-1 flex-col">
        <CardHeader>
          <CardTitle>{current.title}</CardTitle>
          <CardDescription>{current.subtitle}</CardDescription>
          <Stepper count={STEPS.length} active={stepIndex} />
        </CardHeader>

        <ScrollArea className="min-h-0 flex-1">
          <CardContent>
            {current.key === "basics" && (
              <StepBasics
                name={draft.name}
                iso={draft.iso}
                guestArch={draft.guestArch}
                hostArch={hostArch}
                onNameChange={(name) => patch({ name })}
                onIsoChange={(iso) => patch({ iso })}
                onGuestArchChange={(guestArch) => patch({ guestArch })}
              />
            )}
            {current.key === "cpu" && (
              <StepCpuMemory
                cpus={draft.cpus}
                memoryMib={draft.memoryMib}
                hostCores={hostCores}
                onCpusChange={(cpus) => patch({ cpus })}
                onMemoryChange={(memoryMib) => patch({ memoryMib })}
              />
            )}
            {current.key === "storage" && (
              <StepStorage
                diskGib={draft.diskGib}
                onDiskChange={(diskGib) => patch({ diskGib })}
              />
            )}
            {current.key === "network" && (
              <StepNetwork
                value={draft.network}
                onChange={(network) => patch({ network })}
                onValidityChange={setNetworkValid}
              />
            )}
            {current.key === "review" && (
              <StepReview
                name={draft.name.trim()}
                arch={draft.guestArch || hostArch || "x86_64"}
                emulated={
                  hostArch != null &&
                  (draft.guestArch || hostArch) !== hostArch
                }
                cpus={draft.cpus}
                memoryMib={draft.memoryMib}
                diskGib={draft.diskGib}
                network={draft.network}
                iso={draft.iso.trim()}
              />
            )}

            {error && (
              <p className="mt-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                {error}
              </p>
            )}
          </CardContent>
        </ScrollArea>

        <div className="flex items-center justify-between gap-2 border-t border-border p-4">
          <Button
            variant="ghost"
            disabled={busy}
            onClick={
              stepIndex === 0 ? onCancel : () => setStepIndex((i) => i - 1)
            }
          >
            {stepIndex === 0 ? (
              "Cancel"
            ) : (
              <>
                <ArrowLeft className="h-4 w-4" /> Back
              </>
            )}
          </Button>

          {isLast ? (
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                disabled={busy || !networkValid}
                onClick={() => void submit(false)}
              >
                {submitting === "only" ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Plus className="h-4 w-4" />
                )}
                Create only
              </Button>
              <Button
                disabled={busy || !networkValid}
                onClick={() => void submit(true)}
              >
                {submitting === "start" ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Play className="h-4 w-4" />
                )}
                Create &amp; start
              </Button>
            </div>
          ) : (
            <Button
              disabled={!stepValid || busy}
              onClick={() => setStepIndex((i) => i + 1)}
            >
              Next <ArrowRight className="h-4 w-4" />
            </Button>
          )}
        </div>
      </Card>
    </div>
  );
}

/** Compact dot stepper showing progress through the wizard. */
function Stepper({ count, active }: { count: number; active: number }) {
  return (
    <div className="mt-2 flex items-center gap-1.5">
      {Array.from({ length: count }).map((_, i) => (
        <span
          key={i}
          className={
            "h-1.5 flex-1 rounded-full transition-colors " +
            (i <= active ? "bg-primary" : "bg-muted")
          }
        />
      ))}
    </div>
  );
}
