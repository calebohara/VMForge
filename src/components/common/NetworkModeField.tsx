import { Field } from "@/components/common/Field";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { NetworkMode } from "@/lib/ipc";

interface ModeOption {
  value: NetworkMode;
  label: string;
  description: string;
  /** Disabled options are designed-now / implemented-later (Phase 4). */
  disabled?: boolean;
}

const MODES: ModeOption[] = [
  {
    value: "user",
    label: "NAT (user mode)",
    description:
      "Shared with the host via NAT. No setup required; the guest can reach the internet.",
  },
  {
    value: "bridged",
    label: "Bridged",
    description:
      "Guest appears as its own device on your LAN. Requires elevated permissions — arriving in Phase 4.",
    disabled: true,
  },
  {
    value: "host-only",
    label: "Host-only",
    description:
      "Private network between host and guest only. Requires elevated permissions — arriving in Phase 4.",
    disabled: true,
  },
];

/**
 * Network-mode picker shared by the wizard and the hardware editor. Bridged /
 * host-only are shown disabled with an explanation (Phase 4), honoring the
 * "surface limitations, don't hide them" working agreement.
 */
export function NetworkModeField({
  mode,
  onChange,
  disabled,
  id = "vm-network-mode",
}: {
  mode: NetworkMode;
  onChange: (mode: NetworkMode) => void;
  disabled?: boolean;
  id?: string;
}) {
  const selected = MODES.find((m) => m.value === mode) ?? MODES[0];

  return (
    <div className="flex flex-col gap-4">
      <Field label="Network mode" htmlFor={id} hint={selected.description}>
        <Select
          value={mode}
          disabled={disabled}
          onValueChange={(v) => onChange(v as NetworkMode)}
        >
          <SelectTrigger id={id} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {MODES.map((m) => (
              <SelectItem key={m.value} value={m.value} disabled={m.disabled}>
                {m.label}
                {m.disabled && " — Phase 4"}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>

      {(mode === "bridged" || mode === "host-only") && (
        <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-500">
          {selected.label} networking requires elevated permissions and is not
          available yet. NAT (user mode) is recommended for now.
        </p>
      )}
    </div>
  );
}
