import { useEffect, useMemo } from "react";
import { Plus, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Field } from "@/components/common/Field";
import { Input } from "@/components/ui/input";
import { PortForwardRow } from "@/components/common/PortForwardRow";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useNetworkCaps } from "@/hooks/useNetworkCaps";
import type { NetworkConfig, NetworkMode, PortForward } from "@/lib/ipc";
import {
  MAX_PORT_FORWARDS,
  generateMac,
  portForwardWarnings,
  validateMac,
  validatePortForwards,
} from "@/lib/validation";

/** Static, user-facing labels for each network mode. */
export const NETWORK_MODE_LABELS: Record<NetworkMode, string> = {
  user: "NAT (user mode)",
  bridged: "Bridged",
  "host-only": "Host-only",
};

const MODE_ORDER: NetworkMode[] = ["user", "bridged", "host-only"];

const MODE_DESCRIPTIONS: Record<NetworkMode, string> = {
  user: "Shared with the host via NAT. No setup required; the guest can reach the internet.",
  bridged: "Guest appears as its own device on your LAN.",
  "host-only": "Private network between host and guest only.",
};

/** A blank loopback-only TCP forward, ready to edit. */
function emptyForward(): PortForward {
  return { host: NaN, guest: NaN, udp: false, expose_lan: false };
}

/**
 * Shared, fully-controlled networking form used by both the New-VM wizard and
 * the hardware editor. Renders the mode picker, the per-VM MAC, and (in user
 * mode) the NAT port-forward table.
 *
 * - The mode select offers User (always available); Bridged / Host-only are
 *   disabled with the host capability `reason` plus "requires elevated
 *   permissions". A legacy persisted Bridged/Host-only value is shown selected
 *   with an amber note and is NEVER auto-rewritten to User (A2/A10).
 * - Port forwarding is only meaningful in user mode; in other modes the section
 *   is disabled and explained.
 * - `onValidityChange` reports whether the config is valid to SAVE/CREATE
 *   (MAC + port-forward correctness). It does NOT depend on mode availability:
 *   a bridged/host-only config persists (A3) and is only refused at launch.
 */
export function NetworkForm({
  value,
  onChange,
  onValidityChange,
  disabled,
  variant = "editor",
  idPrefix = "net",
}: {
  value: NetworkConfig;
  onChange: (next: NetworkConfig) => void;
  onValidityChange?: (valid: boolean) => void;
  disabled?: boolean;
  variant?: "editor" | "wizard";
  idPrefix?: string;
}) {
  const { caps, loading, error, forMode } = useNetworkCaps();
  const modeId = `${idPrefix}-mode`;
  const macId = `${idPrefix}-mac`;

  const isUserMode = value.mode === "user";

  // MAC validity (optional/blank is valid).
  const macError =
    value.mac && value.mac.trim() !== "" ? validateMac(value.mac.trim()) : null;

  // Per-row port-forward errors + soft warnings (only meaningful in user mode).
  const forwardErrors = useMemo(
    () => validatePortForwards(value.port_forwards),
    [value.port_forwards],
  );
  const forwardWarns = useMemo(
    () => portForwardWarnings(value.port_forwards),
    [value.port_forwards],
  );

  // Port forwards only matter in user mode; over the cap OR any per-row error
  // makes the config invalid (user mode only — fixes the over-cap inversion).
  const overCap = value.port_forwards.length > MAX_PORT_FORWARDS;
  const forwardsValid = !isUserMode
    ? true
    : !overCap && forwardErrors.every((e) => e == null);

  // Validity gates SAVE/CREATE and must NOT depend on mode availability: a
  // bridged/host-only config legitimately persists (A3) — only LAUNCH is
  // refused (engine.start → Error::Config). Mode-unavailability is shown as a
  // non-blocking note, never a save blocker. So validity = MAC + forwards.
  const valid = macError == null && forwardsValid;

  useEffect(() => {
    onValidityChange?.(valid);
  }, [valid, onValidityChange]);

  const selectedDescription = MODE_DESCRIPTIONS[value.mode];
  const legacyPrivileged = value.mode !== "user";

  // ---- mutation helpers ----
  const setMode = (mode: NetworkMode) => onChange({ ...value, mode });
  const setMac = (mac: string | null) => onChange({ ...value, mac });

  const setForward = (index: number, next: PortForward) => {
    const port_forwards = value.port_forwards.map((pf, i) =>
      i === index ? next : pf,
    );
    onChange({ ...value, port_forwards });
  };
  const addForward = () =>
    onChange({ ...value, port_forwards: [...value.port_forwards, emptyForward()] });
  const removeForward = (index: number) =>
    onChange({
      ...value,
      port_forwards: value.port_forwards.filter((_, i) => i !== index),
    });

  const loopbackOnly = caps?.port_forward_loopback_only ?? true;
  const atCap = value.port_forwards.length >= MAX_PORT_FORWARDS;

  return (
    <div className="flex flex-col gap-5">
      {/* ---- Mode ---- */}
      <Field
        label="Network mode"
        htmlFor={modeId}
        hint={selectedDescription}
        error={loading ? null : error}
      >
        <Select
          value={value.mode}
          disabled={disabled}
          onValueChange={(v) => setMode(v as NetworkMode)}
        >
          <SelectTrigger id={modeId} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {MODE_ORDER.map((mode) => {
              const cap = forMode(mode);
              // Default User selectable while caps load; gate the rest until
              // we know whether they are available.
              const available =
                mode === "user" ? cap?.available !== false : cap?.available === true;
              return (
                <SelectItem key={mode} value={mode} disabled={!available}>
                  {NETWORK_MODE_LABELS[mode]}
                  {!available && mode !== "user" && " — requires elevated permissions"}
                </SelectItem>
              );
            })}
          </SelectContent>
        </Select>
      </Field>

      {legacyPrivileged && (
        <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-500">
          {NETWORK_MODE_LABELS[value.mode]} requires elevated permissions and is
          not available yet
          {forMode(value.mode)?.reason ? `: ${forMode(value.mode)?.reason}` : "."}{" "}
          The VM will not launch in this mode — switch to NAT (user mode) to run
          it.
        </p>
      )}

      {/* ---- MAC ---- */}
      <Field
        label="MAC address"
        htmlFor={macId}
        hint="Optional — leave blank to let QEMU assign one automatically."
        error={macError}
      >
        <div className="flex items-center gap-2">
          <Input
            id={macId}
            placeholder="52:54:00:xx:xx:xx (auto)"
            spellCheck={false}
            autoComplete="off"
            disabled={disabled}
            value={value.mac ?? ""}
            aria-invalid={macError != null}
            onChange={(e) => setMac(e.target.value === "" ? null : e.target.value)}
            className="flex-1 font-mono"
          />
          <Button
            type="button"
            variant="outline"
            disabled={disabled}
            onClick={() => setMac(generateMac())}
          >
            <RefreshCw className="h-4 w-4" /> Generate
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={disabled || !value.mac}
            onClick={() => setMac(null)}
          >
            Clear
          </Button>
        </div>
      </Field>

      {/* ---- Port forwarding (user mode only) ---- */}
      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between gap-2">
          <div className="flex flex-col">
            <span className="text-sm font-medium">Port forwarding</span>
            <span className="text-xs text-muted-foreground">
              Forward host ports to the guest over NAT.
            </span>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled || !isUserMode || atCap}
            onClick={addForward}
          >
            <Plus className="h-4 w-4" /> Add forward
          </Button>
        </div>

        {!isUserMode ? (
          <p className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
            Port forwarding applies to NAT (user mode) only. Switch the network
            mode to NAT to forward ports.
          </p>
        ) : (
          <>
            {value.port_forwards.length === 0 ? (
              <p className="rounded-md border border-dashed border-border px-3 py-3 text-center text-xs text-muted-foreground">
                No port forwards. Add one to reach a guest service from the host.
              </p>
            ) : (
              <div className="flex flex-col gap-4">
                {value.port_forwards.map((pf, i) => (
                  <PortForwardRow
                    key={i}
                    index={i}
                    idPrefix={idPrefix}
                    value={pf}
                    disabled={disabled}
                    error={forwardErrors[i]}
                    warning={forwardWarns[i]}
                    onChange={(next) => setForward(i, next)}
                    onRemove={() => removeForward(i)}
                  />
                ))}
              </div>
            )}

            {overCap && (
              <p className="text-xs text-destructive">
                At most {MAX_PORT_FORWARDS} port forwards are allowed.
              </p>
            )}

            <p className="text-xs text-muted-foreground">
              {loopbackOnly
                ? "Forwards bind 127.0.0.1 (loopback) by default — only this machine can reach them. Use “Expose to LAN” per forward to bind all interfaces."
                : "Forwards bind all interfaces by default."}
            </p>
          </>
        )}
      </div>

      {variant === "editor" && (
        <p className="text-xs text-muted-foreground">
          Network changes apply the next time the VM launches.
        </p>
      )}
    </div>
  );
}
