import { useEffect, useMemo, useState } from "react";
import { Camera, Loader2, Lock, RotateCcw, Save } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Skeleton } from "@/components/ui/skeleton";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Field } from "@/components/common/Field";
import { IsoPicker } from "@/components/common/IsoPicker";
import { Input } from "@/components/ui/input";
import { StatusBadge } from "@/components/common/StatusBadge";
import { ProcessorsTab } from "@/components/editor/tabs/ProcessorsTab";
import { MemoryTab } from "@/components/editor/tabs/MemoryTab";
import { NetworkTab } from "@/components/editor/tabs/NetworkTab";
import { SharedFoldersTab } from "@/components/editor/tabs/SharedFoldersTab";
import { getVm, updateVm } from "@/lib/ipc";
import type { NetworkConfig, SharedFolder, VmConfig, VmState } from "@/lib/ipc";
import {
  normalizeNetwork,
  normalizeSharedFolders,
  validateCpus,
  validateMemoryMib,
  validateVmName,
} from "@/lib/validation";

interface EditorDraft {
  name: string;
  cpus: number;
  memoryMib: number;
  /** Full network config (A10): round-trips mode + MAC + port forwards. */
  network: NetworkConfig;
  iso: string;
  /** virtio-9p shared folders (Phase 5). */
  shared: SharedFolder[];
}

function draftFromConfig(c: VmConfig): EditorDraft {
  return {
    name: c.name,
    cpus: c.hardware.cpus,
    memoryMib: c.hardware.memory_mib,
    // Deep-copy so editing the draft never mutates the loaded original.
    network: {
      mode: c.network.mode,
      mac: c.network.mac,
      port_forwards: c.network.port_forwards.map((pf) => ({ ...pf })),
    },
    iso: c.iso ?? "",
    shared: (c.shared_folders ?? []).map((sf) => ({ ...sf })),
  };
}

/** Edits are only allowed while the VM is stopped/defined (decision A.7). */
export function isEditable(state: VmState): boolean {
  return state === "stopped" || state === "defined";
}

/**
 * Hardware editor for a STOPPED VM (spec §D). Tabs: Processors / Memory /
 * Network — NO disk tab (Phase 3). Plus a top-level name + ISO. All controls are
 * disabled (with an explanatory tooltip) unless the VM is stopped/defined. Dirty
 * tracking drives Save/Discard; Save calls `update_vm` then returns to library.
 */
export function HardwareEditorView({
  vmId,
  state,
  hostCores,
  onClose,
  onSaved,
  onOpenSnapshots,
}: {
  vmId: string;
  /** Live state from the library poll; gates whether fields are editable. */
  state: VmState;
  hostCores: number | null;
  onClose: () => void;
  /** Called after a successful save (parent refreshes + returns to library). */
  onSaved: () => void;
  /** Switch to this VM's snapshot manager. */
  onOpenSnapshots: () => void;
}) {
  const [original, setOriginal] = useState<VmConfig | null>(null);
  const [draft, setDraft] = useState<EditorDraft | null>(null);
  const [networkValid, setNetworkValid] = useState(true);
  const [sharedValid, setSharedValid] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // Load the config once.
  useEffect(() => {
    let cancelled = false;
    getVm(vmId)
      .then((c) => {
        if (cancelled) return;
        setOriginal(c);
        setDraft(draftFromConfig(c));
      })
      .catch((e) => {
        if (!cancelled) setLoadError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [vmId]);

  const editable = isEditable(state);
  const locked = !editable;

  const patch = (p: Partial<EditorDraft>) =>
    setDraft((d) => (d ? { ...d, ...p } : d));

  const dirty = useMemo(() => {
    if (!original || !draft) return false;
    const base = draftFromConfig(original);
    return (
      base.name !== draft.name ||
      base.cpus !== draft.cpus ||
      base.memoryMib !== draft.memoryMib ||
      JSON.stringify(base.network) !== JSON.stringify(draft.network) ||
      base.iso !== draft.iso ||
      JSON.stringify(base.shared) !== JSON.stringify(draft.shared)
    );
  }, [original, draft]);

  const nameError = draft ? validateVmName(draft.name) : null;
  const formValid =
    draft != null &&
    nameError === null &&
    validateCpus(draft.cpus) === null &&
    validateMemoryMib(draft.memoryMib) === null &&
    networkValid &&
    sharedValid;

  const discard = () => {
    if (original) setDraft(draftFromConfig(original));
    setSaveError(null);
  };

  const save = async () => {
    if (!draft || !formValid) return;
    setSaving(true);
    setSaveError(null);
    try {
      await updateVm(vmId, {
        name: draft.name.trim(),
        hardware: { cpus: draft.cpus, memory_mib: draft.memoryMib },
        network: normalizeNetwork(draft.network),
        iso: draft.iso.trim() ? draft.iso.trim() : null,
        shared_folders: normalizeSharedFolders(draft.shared),
      });
      onSaved();
    } catch (e) {
      setSaveError(String(e));
      setSaving(false);
    }
  };

  if (loadError) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-4 p-10 text-center">
        <p className="max-w-sm text-sm text-destructive">{loadError}</p>
        <Button variant="outline" onClick={onClose}>
          Back to library
        </Button>
      </div>
    );
  }

  if (!draft) {
    return (
      <div className="mx-auto w-full max-w-2xl space-y-4 p-5">
        <Skeleton className="h-10 w-1/2" />
        <Skeleton className="h-64 w-full rounded-xl" />
      </div>
    );
  }

  return (
    <div className="mx-auto flex min-h-0 w-full max-w-2xl flex-1 flex-col p-5">
      <Card className="flex min-h-0 flex-1 flex-col">
        <CardHeader>
          <div className="flex items-center gap-3">
            <CardTitle className="mr-auto truncate" title={draft.name}>
              Edit hardware
            </CardTitle>
            <Button
              variant="outline"
              size="sm"
              onClick={onOpenSnapshots}
              disabled={saving}
            >
              <Camera className="h-4 w-4" /> Snapshots
            </Button>
            <StatusBadge state={state} />
          </div>
          {locked && (
            <div className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-500">
              <Lock className="h-3.5 w-3.5 shrink-0" />
              Hardware can only be edited while the VM is stopped. Shut it down to
              make changes.
            </div>
          )}
        </CardHeader>

        <ScrollArea className="min-h-0 flex-1">
          <CardContent className="flex flex-col gap-5">
            <Field
              label="Name"
              htmlFor="edit-name"
              error={editable ? nameError : null}
            >
              <Input
                id="edit-name"
                value={draft.name}
                disabled={locked}
                onChange={(e) => patch({ name: e.target.value })}
              />
            </Field>

            <Tabs defaultValue="processors">
              <TabsList>
                <TabsTrigger value="processors">Processors</TabsTrigger>
                <TabsTrigger value="memory">Memory</TabsTrigger>
                <TabsTrigger value="network">Network</TabsTrigger>
                <TabsTrigger value="shared">Shared folders</TabsTrigger>
              </TabsList>

              <TabsContent value="processors" className="pt-4">
                <ProcessorsTab
                  cpus={draft.cpus}
                  hostCores={hostCores}
                  disabled={locked}
                  onChange={(cpus) => patch({ cpus })}
                />
              </TabsContent>

              <TabsContent value="memory" className="pt-4">
                <MemoryTab
                  memoryMib={draft.memoryMib}
                  disabled={locked}
                  onChange={(memoryMib) => patch({ memoryMib })}
                />
              </TabsContent>

              <TabsContent value="network" className="pt-4">
                <NetworkTab
                  value={draft.network}
                  disabled={locked}
                  onChange={(network) => patch({ network })}
                  onValidityChange={setNetworkValid}
                />
              </TabsContent>

              <TabsContent value="shared" className="pt-4">
                <SharedFoldersTab
                  value={draft.shared}
                  disabled={locked}
                  onChange={(shared) => patch({ shared })}
                  onValidityChange={setSharedValid}
                />
              </TabsContent>
            </Tabs>

            <Field
              label="Installer ISO"
              hint="Attach or change the boot disc image."
            >
              <IsoPicker
                value={draft.iso}
                disabled={locked}
                onChange={(iso) => patch({ iso })}
              />
            </Field>

            {saveError && (
              <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                {saveError}
              </p>
            )}
          </CardContent>
        </ScrollArea>

        <div className="flex items-center justify-between gap-2 border-t border-border p-4">
          <Button variant="ghost" onClick={onClose} disabled={saving}>
            {dirty ? "Cancel" : "Back to library"}
          </Button>
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              onClick={discard}
              disabled={!dirty || saving || locked}
            >
              <RotateCcw className="h-4 w-4" /> Discard
            </Button>
            <SaveButton
              disabled={!dirty || !formValid || saving || locked}
              saving={saving}
              locked={locked}
              onSave={() => void save()}
            />
          </div>
        </div>
      </Card>
    </div>
  );
}

function SaveButton({
  disabled,
  saving,
  locked,
  onSave,
}: {
  disabled: boolean;
  saving: boolean;
  locked: boolean;
  onSave: () => void;
}) {
  const btn = (
    <Button disabled={disabled} onClick={onSave}>
      {saving ? (
        <Loader2 className="h-4 w-4 animate-spin" />
      ) : (
        <Save className="h-4 w-4" />
      )}
      Save changes
    </Button>
  );
  if (!locked) return btn;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="inline-flex">{btn}</span>
      </TooltipTrigger>
      <TooltipContent>Stop the VM to edit its hardware.</TooltipContent>
    </Tooltip>
  );
}
