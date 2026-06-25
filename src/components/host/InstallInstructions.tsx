import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, Terminal } from "lucide-react";
import { Button } from "@/components/ui/button";
import { installGuide } from "@/lib/hostStatus";

/**
 * Per-OS QEMU install guide for the first-run gate: a heading, copy-pasteable
 * commands, and a docs link. The docs link opens in the user's browser via
 * `tauri-plugin-opener`; if that fails (or in a non-Tauri test context) we fall
 * back to rendering the URL as selectable text so it's never a dead end.
 */
export function InstallInstructions({ os }: { os: string }) {
  const guide = installGuide(os);
  // Becomes true if openUrl throws — we then show the raw, selectable URL.
  const [openerFailed, setOpenerFailed] = useState(false);

  const openDocs = async () => {
    try {
      await openUrl(guide.docsUrl);
    } catch {
      setOpenerFailed(true);
    }
  };

  return (
    <div className="space-y-3">
      <h3 className="text-sm font-semibold">{guide.heading}</h3>

      <ul className="space-y-2">
        {guide.steps.map((step, i) => (
          <li key={i} className="space-y-1">
            <p className="text-sm text-muted-foreground">{step.description}</p>
            {step.command && (
              <code className="flex items-center gap-2 rounded-md bg-muted px-3 py-2 font-mono text-xs select-all">
                <Terminal className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                {step.command}
              </code>
            )}
          </li>
        ))}
      </ul>

      <div className="flex flex-wrap items-center gap-2">
        <Button variant="outline" size="sm" onClick={() => void openDocs()}>
          <ExternalLink className="h-3.5 w-3.5" /> QEMU download page
        </Button>
        {openerFailed && (
          <span className="font-mono text-xs text-muted-foreground select-all">
            {guide.docsUrl}
          </span>
        )}
      </div>
    </div>
  );
}
