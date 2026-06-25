import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import type { PortForward } from "@/lib/ipc";

/** Parse a number-input value, returning NaN for empty/garbage. */
function parsePort(raw: string): number {
  if (raw.trim() === "") return Number.NaN;
  return Number(raw);
}

/**
 * A single port-forward row: host port, guest port, TCP/UDP protocol, an
 * "expose to LAN" toggle, and a remove button. Fully controlled. Inline range
 * and duplicate errors are passed in via {@link error}; a soft (non-blocking)
 * warning is passed via {@link warning}.
 */
export function PortForwardRow({
  value,
  index,
  idPrefix = "pf",
  disabled,
  error,
  warning,
  onChange,
  onRemove,
}: {
  value: PortForward;
  /** Zero-based row index, used to build stable input ids and aria-labels. */
  index: number;
  idPrefix?: string;
  disabled?: boolean;
  /** Inline blocking error for this row (range / duplicate). */
  error?: string | null;
  /** Inline soft warning for this row (e.g. privileged port). */
  warning?: string | null;
  onChange: (next: PortForward) => void;
  onRemove: () => void;
}) {
  const rowId = `${idPrefix}-row-${index}`;
  const hostId = `${rowId}-host`;
  const guestId = `${rowId}-guest`;
  const protoId = `${rowId}-proto`;
  const invalid = error != null;

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-end gap-2">
        <div className="flex flex-1 flex-col gap-1">
          <Label htmlFor={hostId} className="text-[11px] text-muted-foreground">
            Host port
          </Label>
          <Input
            id={hostId}
            type="number"
            min={1}
            max={65535}
            inputMode="numeric"
            aria-label={`Host port for forward ${index + 1}`}
            aria-invalid={invalid}
            disabled={disabled}
            value={Number.isNaN(value.host) ? "" : value.host}
            onChange={(e) =>
              onChange({ ...value, host: parsePort(e.target.value) })
            }
            className={cn(invalid && "border-destructive")}
          />
        </div>

        <span className="pb-2 text-muted-foreground">→</span>

        <div className="flex flex-1 flex-col gap-1">
          <Label htmlFor={guestId} className="text-[11px] text-muted-foreground">
            Guest port
          </Label>
          <Input
            id={guestId}
            type="number"
            min={1}
            max={65535}
            inputMode="numeric"
            aria-label={`Guest port for forward ${index + 1}`}
            aria-invalid={invalid}
            disabled={disabled}
            value={Number.isNaN(value.guest) ? "" : value.guest}
            onChange={(e) =>
              onChange({ ...value, guest: parsePort(e.target.value) })
            }
            className={cn(invalid && "border-destructive")}
          />
        </div>

        <div className="flex flex-col gap-1">
          <Label htmlFor={protoId} className="text-[11px] text-muted-foreground">
            Protocol
          </Label>
          <Select
            value={value.udp ? "udp" : "tcp"}
            disabled={disabled}
            onValueChange={(v) => onChange({ ...value, udp: v === "udp" })}
          >
            <SelectTrigger
              id={protoId}
              className="w-24"
              aria-label={`Protocol for forward ${index + 1}`}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="tcp">TCP</SelectItem>
              <SelectItem value="udp">UDP</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <Button
          type="button"
          variant="ghost"
          size="icon"
          disabled={disabled}
          aria-label={`Remove forward ${index + 1}`}
          onClick={onRemove}
          className="mb-0.5 shrink-0 text-muted-foreground"
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <label className="flex items-center gap-2 text-xs text-muted-foreground">
        <input
          type="checkbox"
          className="size-3.5 accent-primary"
          disabled={disabled}
          checked={value.expose_lan}
          aria-label={`Expose forward ${index + 1} to the LAN`}
          onChange={(e) =>
            onChange({ ...value, expose_lan: e.target.checked })
          }
        />
        Expose to LAN (bind all interfaces instead of loopback)
      </label>

      {error ? (
        <p className="text-xs text-destructive">{error}</p>
      ) : warning ? (
        <p className="text-xs text-amber-500">{warning}</p>
      ) : null}
    </div>
  );
}
