import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { toast } from "sonner";

/**
 * Auto-update entry point (spec §D, decision D5 / D8).
 *
 * WIRED BUT NOT ACTIVATED. The committed `tauri.conf.json` ships
 * `bundle.createUpdaterArtifacts: false` plus a placeholder `plugins.updater`
 * pubkey, so no production build emits signed update artifacts and `check()`
 * has nothing real to resolve against. This module is therefore inert until a
 * release flips `createUpdaterArtifacts` true (via the `--config
 * src-tauri/tauri.release.conf.json` patch in release.yml), a real keypair is
 * configured, and a `latest.json` feed is published.
 *
 * Until then, invoking `checkForUpdates()` exercises the full flow but will
 * almost always land on "you're up to date" or surface a (toasted) error from
 * the plugin — both harmless. The flow is roll-forward only and, per D8,
 * auto-installs on macOS + Windows; Linux falls back to a manual download.
 */

/** The update lifecycle, surfaced to the UI for an optional progress affordance. */
export type UpdateStatus =
  | { kind: "checking" }
  | { kind: "up-to-date" }
  | { kind: "available"; version: string }
  | { kind: "downloading"; downloaded: number; total: number | null }
  | { kind: "installing" }
  | { kind: "error"; message: string };

export interface CheckForUpdatesOptions {
  /**
   * Optional progress sink for a richer UI (e.g. a progress bar). The function
   * still toasts user-facing outcomes regardless of whether this is supplied.
   */
  onStatus?: (status: UpdateStatus) => void;
  /**
   * Skip the relaunch after a successful install (used in tests, or when the
   * caller wants to defer the restart). Defaults to relaunching immediately.
   */
  skipRelaunch?: boolean;
}

/**
 * Check for an update and, if one is available, download + install it with
 * progress reporting, then relaunch into the new version.
 *
 * Errors are caught and toasted (never thrown) so a wired-but-not-activated
 * updater can't crash the UI. Returns `true` when an update was installed,
 * `false` when already up to date, and `false` on error.
 */
export async function checkForUpdates(
  opts: CheckForUpdatesOptions = {},
): Promise<boolean> {
  const { onStatus, skipRelaunch = false } = opts;

  try {
    onStatus?.({ kind: "checking" });
    const update = await check();

    // `check()` resolves to null when there is no newer version. (The
    // `Update.available` flag is deprecated — null is the source of truth.)
    if (update === null) {
      onStatus?.({ kind: "up-to-date" });
      toast.info("VMForge is up to date.");
      return false;
    }

    onStatus?.({ kind: "available", version: update.version });
    toast.info(`Update ${update.version} found — downloading…`);

    let total: number | null = null;
    let downloaded = 0;

    await update.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          total = event.data.contentLength ?? null;
          downloaded = 0;
          onStatus?.({ kind: "downloading", downloaded, total });
          break;
        case "Progress":
          downloaded += event.data.chunkLength;
          onStatus?.({ kind: "downloading", downloaded, total });
          break;
        case "Finished":
          onStatus?.({ kind: "installing" });
          break;
      }
    });

    toast.success(`Update ${update.version} installed — restarting…`);

    if (!skipRelaunch) {
      // Roll-forward only (D8): relaunch into the freshly installed version.
      await relaunch();
    }
    return true;
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    onStatus?.({ kind: "error", message });
    toast.error(`Update check failed: ${message}`);
    return false;
  }
}
