import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { HostCapabilities } from "@/lib/ipc";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

import { useHostCaps } from "@/hooks/useHostCaps";

function caps(over: Partial<HostCapabilities> = {}): HostCapabilities {
  return {
    os: "macos",
    arch: "aarch64",
    preferred_accelerator: "hvf",
    available_accelerators: ["hvf", "tcg"],
    hardware_accelerated: true,
    qemu_img: { name: "qemu-img", present: true, version: "11.0.1" },
    system_binaries: [
      { name: "qemu-system-aarch64", present: true, version: "11.0.1" },
      { name: "qemu-system-x86_64", present: true, version: "11.0.1" },
    ],
    network: { modes: [], port_forward_loopback_only: true },
    warnings: [],
    ...over,
  };
}

beforeEach(() => {
  invoke.mockReset();
});

describe("useHostCaps", () => {
  it("probes once on mount and clears loading", async () => {
    invoke.mockResolvedValue(caps());
    const { result } = renderHook(() => useHostCaps());

    expect(result.current.loading).toBe(true);
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.caps?.os).toBe("macos");
    expect(invoke).toHaveBeenCalledWith("probe_host");
  });

  it("refresh() re-probes, toggling `refreshing` (not `loading`)", async () => {
    // Initial probe: QEMU missing. Re-probe: QEMU present.
    invoke
      .mockResolvedValueOnce(caps({ qemu_img: { name: "qemu-img", present: false, version: null } }))
      .mockResolvedValueOnce(caps());

    const { result } = renderHook(() => useHostCaps());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.caps?.qemu_img.present).toBe(false);

    await act(async () => {
      await result.current.refresh();
    });

    // loading stays false across a refresh; refreshing settled back to false.
    expect(result.current.loading).toBe(false);
    expect(result.current.refreshing).toBe(false);
    expect(result.current.caps?.qemu_img.present).toBe(true);
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("refresh() resolves with the fresh caps", async () => {
    invoke.mockResolvedValue(caps());
    const { result } = renderHook(() => useHostCaps());
    await waitFor(() => expect(result.current.loading).toBe(false));

    let returned: HostCapabilities | null = null;
    await act(async () => {
      returned = await result.current.refresh();
    });
    expect(returned).not.toBeNull();
    expect(returned!.os).toBe("macos");
  });

  it("clears a prior error when a later probe succeeds", async () => {
    invoke.mockRejectedValueOnce("boom").mockResolvedValueOnce(caps());
    const { result } = renderHook(() => useHostCaps());
    await waitFor(() => expect(result.current.error).toBe("boom"));

    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.error).toBeNull();
    expect(result.current.caps?.os).toBe("macos");
  });

  it("surfaces a refresh failure via error without throwing", async () => {
    invoke.mockResolvedValueOnce(caps()).mockRejectedValueOnce("probe failed");
    const { result } = renderHook(() => useHostCaps());
    await waitFor(() => expect(result.current.loading).toBe(false));

    let returned: HostCapabilities | null = caps();
    await act(async () => {
      returned = await result.current.refresh();
    });
    expect(returned).toBeNull();
    expect(result.current.error).toBe("probe failed");
  });
});
