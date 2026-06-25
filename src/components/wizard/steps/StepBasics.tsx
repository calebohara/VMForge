import { Field } from "@/components/common/Field";
import { IsoPicker } from "@/components/common/IsoPicker";
import { Input } from "@/components/ui/input";
import { validateVmName } from "@/lib/validation";

/**
 * Step 1 — Basics: VM name (validated) and an optional installer ISO. VMForge
 * builds x86-64 VMs (Windows / Linux x86 ISOs).
 */
export function StepBasics({
  name,
  iso,
  onNameChange,
  onIsoChange,
}: {
  name: string;
  iso: string;
  onNameChange: (name: string) => void;
  onIsoChange: (iso: string) => void;
}) {
  const nameError = name.length > 0 ? validateVmName(name) : null;

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
          placeholder="e.g. Windows 11"
          onChange={(e) => onNameChange(e.target.value)}
        />
      </Field>

      <Field
        label="Installer ISO (optional)"
        hint="Attach a disc image to boot an installer. You can add or change this later."
      >
        <IsoPicker value={iso} onChange={onIsoChange} />
      </Field>
    </div>
  );
}
