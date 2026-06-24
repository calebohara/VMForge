import { cn } from "@/lib/utils";
import { Field } from "@/components/common/Field";
import { Input } from "@/components/ui/input";
import { Slider } from "@/components/ui/slider";
import { clamp } from "@/lib/validation";

/**
 * Numeric field combining a slider and a number input, with an optional
 * "host headroom" caption (e.g. "Host: 10 cores"). Surfaces over-allocation
 * honestly via `softMax` (the recommended ceiling) without hard-blocking.
 */
export function LimitField({
  label,
  value,
  min,
  max,
  step = 1,
  unit,
  softMax,
  softMaxLabel,
  disabled,
  error,
  hint,
  onChange,
  id,
  className,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  /** Recommended ceiling (e.g. host cores). Exceeding it shows a warning. */
  softMax?: number;
  /** Caption rendered under the control (e.g. "Host: 10 cores"). */
  softMaxLabel?: string;
  disabled?: boolean;
  error?: string | null;
  hint?: string;
  onChange: (value: number) => void;
  id?: string;
  className?: string;
}) {
  const overAllocated = softMax != null && value > softMax;
  const computedHint =
    hint ??
    (softMaxLabel
      ? overAllocated
        ? `${softMaxLabel} — over-allocating may degrade host performance.`
        : softMaxLabel
      : undefined);

  return (
    <Field
      label={unit ? `${label} (${unit})` : label}
      htmlFor={id}
      error={error}
      hint={!error ? computedHint : undefined}
      className={className}
    >
      <div className="flex items-center gap-3">
        <Slider
          value={[clamp(value, min, max)]}
          min={min}
          max={max}
          step={step}
          disabled={disabled}
          onValueChange={(v) => onChange(v[0] ?? min)}
          className="flex-1"
        />
        <Input
          id={id}
          type="number"
          min={min}
          max={max}
          step={step}
          disabled={disabled}
          value={value}
          onChange={(e) => {
            const n = Number(e.target.value);
            onChange(Number.isNaN(n) ? min : clamp(n, min, max));
          }}
          className={cn(
            "w-24",
            overAllocated && "border-amber-500 focus-visible:border-amber-500",
          )}
        />
      </div>
    </Field>
  );
}
