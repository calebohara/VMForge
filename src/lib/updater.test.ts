import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock the Tauri updater/process plugins + sonner before importing the module
// under test. The updater is wired-but-not-activated, so these stand in for the
// (inert) real plugins.
const check = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: (...args: unknown[]) => check(...args),
}));

const relaunch = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: (...args: unknown[]) => relaunch(...args),
}));

const toast = vi.hoisted(() => ({
  info: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}));
vi.mock("sonner", () => ({ toast }));

import { checkForUpdates, type UpdateStatus } from "@/lib/updater";

beforeEach(() => {
  check.mockReset();
  relaunch.mockReset();
  toast.info.mockReset();
  toast.success.mockReset();
  toast.error.mockReset();
});

describe("checkForUpdates", () => {
  it("reports up-to-date when check() resolves null", async () => {
    check.mockResolvedValue(null);
    const statuses: UpdateStatus[] = [];

    const installed = await checkForUpdates({
      onStatus: (s) => statuses.push(s),
    });

    expect(installed).toBe(false);
    expect(relaunch).not.toHaveBeenCalled();
    expect(toast.info).toHaveBeenCalledWith("VMForge is up to date.");
    expect(statuses.map((s) => s.kind)).toEqual(["checking", "up-to-date"]);
  });

  it("downloads, installs, and relaunches when an update is available", async () => {
    const downloadAndInstall = vi.fn(
      (onEvent: (e: unknown) => void) => {
        onEvent({ event: "Started", data: { contentLength: 100 } });
        onEvent({ event: "Progress", data: { chunkLength: 40 } });
        onEvent({ event: "Progress", data: { chunkLength: 60 } });
        onEvent({ event: "Finished" });
        return Promise.resolve();
      },
    );
    check.mockResolvedValue({ version: "0.2.0", downloadAndInstall });
    relaunch.mockResolvedValue(undefined);

    const statuses: UpdateStatus[] = [];
    const installed = await checkForUpdates({
      onStatus: (s) => statuses.push(s),
    });

    expect(installed).toBe(true);
    expect(downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(relaunch).toHaveBeenCalledTimes(1);
    expect(toast.success).toHaveBeenCalledWith(
      "Update 0.2.0 installed — restarting…",
    );

    // Accumulated download progress + the lifecycle transitions.
    expect(statuses.map((s) => s.kind)).toEqual([
      "checking",
      "available",
      "downloading",
      "downloading",
      "downloading",
      "installing",
    ]);
    const last = statuses.filter((s) => s.kind === "downloading").at(-1);
    expect(last).toEqual({ kind: "downloading", downloaded: 100, total: 100 });
  });

  it("honors skipRelaunch (no restart)", async () => {
    check.mockResolvedValue({
      version: "0.2.0",
      downloadAndInstall: vi.fn().mockResolvedValue(undefined),
    });

    const installed = await checkForUpdates({ skipRelaunch: true });

    expect(installed).toBe(true);
    expect(relaunch).not.toHaveBeenCalled();
  });

  it("catches and toasts errors without throwing (inert-safe)", async () => {
    check.mockRejectedValue(new Error("no endpoint configured"));
    const statuses: UpdateStatus[] = [];

    const installed = await checkForUpdates({
      onStatus: (s) => statuses.push(s),
    });

    expect(installed).toBe(false);
    expect(toast.error).toHaveBeenCalledWith(
      "Update check failed: no endpoint configured",
    );
    expect(statuses.at(-1)).toEqual({
      kind: "error",
      message: "no endpoint configured",
    });
  });
});
