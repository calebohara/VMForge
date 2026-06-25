import { useCallback } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

/**
 * Host-directory picker (Phase 5). Sibling of {@link IsoPicker} but selects a
 * directory via the Tauri dialog plugin (no direct FS access). `value` is the
 * absolute path (or empty); `onChange` receives the new path.
 */
export function DirectoryPicker({
  value,
  onChange,
  disabled,
  className,
  placeholder = "Choose a host folder…",
  ariaLabel,
}: {
  value: string;
  onChange: (path: string) => void;
  disabled?: boolean;
  className?: string;
  placeholder?: string;
  /** Accessible label for the path input. */
  ariaLabel?: string;
}) {
  const browse = useCallback(async () => {
    const selected = await open({
      directory: true,
      multiple: false,
    });
    if (typeof selected === "string") onChange(selected);
  }, [onChange]);

  return (
    <div className={cn("flex gap-2", className)}>
      <Input
        value={value}
        placeholder={placeholder}
        aria-label={ariaLabel}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        className="flex-1"
      />
      {value && !disabled && (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Clear folder"
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
