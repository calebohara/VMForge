import { Field } from "@/components/common/Field";
import { IsoPicker } from "@/components/common/IsoPicker";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { validateVmName } from "@/lib/validation";

/** Display labels for the guest architectures VMForge can launch. */
export const ARCH_LABELS: Record<string, string> = {
  x86_64: "x86-64 (Intel / AMD)",
  aarch64: "ARM64 (aarch64)",
};
const ARCH_OPTIONS = ["x86_64", "aarch64"] as const;

/**
 * Step 1 — Basics: VM name (validated), guest architecture, and an optional
 * installer ISO. The architecture defaults to the host's (hardware-accelerated);
 * choosing the other architecture is allowed but runs under TCG emulation, which
 * we surface honestly. Arch is fixed at create time (it matches the installed
 * OS), so it is not editable later.
 */
export function StepBasics({
  name,
  iso,
  guestArch,
  hostArch,
  onNameChange,
  onIsoChange,
  onGuestArchChange,
}: {
  name: string;
  iso: string;
  /** The effective guest arch ("" falls back to host). */
  guestArch: string;
  /** Host arch from the capability probe, or null while it loads. */
  hostArch: string | null;
  onNameChange: (name: string) => void;
  onIsoChange: (iso: string) => void;
  onGuestArchChange: (arch: string) => void;
}) {
  const nameError = name.length > 0 ? validateVmName(name) : null;
  // Normalize to a known, selectable option so the Select always shows a value
  // even on an unrecognized host arch (and never submits an empty arch).
  const hostOption =
    hostArch === "aarch64" || hostArch === "x86_64" ? hostArch : "x86_64";
  const effectiveArch =
    guestArch === "aarch64" || guestArch === "x86_64" ? guestArch : hostOption;
  // Only claim emulation once the host arch is actually known.
  const emulated = hostArch != null && effectiveArch !== hostOption;

  return (
    <div className="flex flex-col gap-5">
      <Field
        label="Name"
        htmlFor="vm-name"
        error={nameError}
        hint="A friendly name. The on-disk folder is derived from it."
      >
        <Input
          id="vm-name"
          autoFocus
          value={name}
          placeholder="e.g. Alpine VM"
          onChange={(e) => onNameChange(e.target.value)}
        />
      </Field>

      <Field
        label="Architecture"
        htmlFor="vm-arch"
        hint={
          emulated
            ? `Emulated: a ${ARCH_LABELS[effectiveArch] ?? effectiveArch} guest doesn't match this ${ARCH_LABELS[hostOption] ?? hostOption} host, so it runs under TCG software emulation and will be slow.`
            : "Matches this host — hardware-accelerated. Fixed once the VM is created."
        }
      >
        <Select value={effectiveArch} onValueChange={onGuestArchChange}>
          <SelectTrigger id="vm-arch" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {ARCH_OPTIONS.map((a) => (
              <SelectItem key={a} value={a}>
                {ARCH_LABELS[a]}
                {hostArch === a
                  ? " — native"
                  : hostArch != null
                    ? " — emulated"
                    : ""}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>

      <Field
        label="Installer ISO (optional)"
        hint="Attach a disc image to boot an installer. Match it to the architecture above."
      >
        <IsoPicker value={iso} onChange={onIsoChange} />
      </Field>
    </div>
  );
}
