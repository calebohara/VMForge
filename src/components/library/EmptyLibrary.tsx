import { Plus, Server } from "lucide-react";
import { Button } from "@/components/ui/button";

/** Shown when the library has no VMs yet. */
export function EmptyLibrary({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 p-10 text-center">
      <div className="flex h-16 w-16 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
        <Server className="h-8 w-8" />
      </div>
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">No virtual machines yet</h2>
        <p className="max-w-sm text-sm text-muted-foreground">
          Create your first VM to boot an ISO, install an OS, and manage it from
          your library.
        </p>
      </div>
      <Button onClick={onCreate}>
        <Plus className="h-4 w-4" /> New virtual machine
      </Button>
    </div>
  );
}
