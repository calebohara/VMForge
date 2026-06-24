import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { VmListItem } from "@/lib/ipc";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

import { useVmLibrary } from "@/hooks/useVmLibrary";

function vm(id: string, name: string): VmListItem {
  return {
    id,
    name,
    state: "defined",
    accelerator: "hvf",
    emulated: false,
    cpus: 2,
    memory_mib: 2048,
    iso: null,
  };
}

// Helper to drive document.hidden + visibilitychange in jsdom.
function setHidden(hidden: boolean) {
  Object.defineProperty(document, "hidden", {
    configurable: true,
    get: () => hidden,
  });
  document.dispatchEvent(new Event("visibilitychange"));
}

beforeEach(() => {
  invoke.mockReset();
  setHidden(false);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useVmLibrary", () => {
  it("renders VMs from the mocked list_vms", async () => {
    invoke.mockResolvedValue([vm("1", "alpha")]);
    const { result } = renderHook(() => useVmLibrary());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.vms).toHaveLength(1);
    expect(result.current.vms[0].name).toBe("alpha");
    expect(invoke).toHaveBeenCalledWith("list_vms");
  });

  it("polls on a 2s interval and pauses while hidden", async () => {
    vi.useFakeTimers();
    invoke.mockResolvedValue([]);

    renderHook(() => useVmLibrary());
    // Initial fetch fires synchronously on mount.
    expect(invoke).toHaveBeenCalledTimes(1);

    // One poll tick.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(invoke).toHaveBeenCalledTimes(2);

    // Hide the document: polling stops (no new invokes across two intervals).
    act(() => setHidden(true));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(4000);
    });
    expect(invoke).toHaveBeenCalledTimes(2);

    // Becoming visible again triggers an immediate refetch + resumes polling.
    act(() => setHidden(false));
    expect(invoke).toHaveBeenCalledTimes(3);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(invoke).toHaveBeenCalledTimes(4);
  });

  it("refresh() forces an immediate fetch", async () => {
    invoke.mockResolvedValue([]);
    const { result } = renderHook(() => useVmLibrary());
    await waitFor(() => expect(result.current.loading).toBe(false));
    const before = invoke.mock.calls.length;
    await act(async () => {
      await result.current.refresh();
    });
    expect(invoke.mock.calls.length).toBe(before + 1);
  });

  it("surfaces errors without throwing", async () => {
    invoke.mockRejectedValue("boom");
    const { result } = renderHook(() => useVmLibrary());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.error).toBe("boom");
    expect(result.current.vms).toEqual([]);
  });
});
