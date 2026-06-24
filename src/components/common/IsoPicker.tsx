import { useCallback } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

/**
 * ISO/disc-image picker. Uses the Tauri dialog plugin (no direct FS access).
 * `value` is the absolute path (or empty); `onChange` receives the new path.
 */
export function IsoPicker({
  value,
  onChange,
  disabled,
  className,
}: {
  value: string;
  onChange: (path: string) => void;
  disabled?: boolean;
  className?: string;
}) {
  const browse = useCallback(async () => {
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Disc image", extensions: ["iso", "img"] }],
    });
    if (typeof selected === "string") onChange(selected);
  }, [onChange]);

  return (
    <div className={cn("flex gap-2", className)}>
      <Input
        value={value}
        placeholder="Choose an .iso…"
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        className="flex-1"
      />
      {value && !disabled && (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Clear ISO"
          onClick={() => onChange("")}
        >
          <X className="h-4 w-4" />
        </Button>
      )}
      <Button
        type="button"
        variant="outline"
        onClick={browse}
        disabled={disabled}
      >
        <FolderOpen className="h-4 w-4" /> Browse
      </Button>
    </div>
  );
}
