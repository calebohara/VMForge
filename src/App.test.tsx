import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import type { HostCapabilities } from "@/lib/ipc";
import type { UseHostCaps } from "@/hooks/useHostCaps";

// App pulls in a deep component tree; mock the two data hooks so the test drives
// only the gate-wiring branch, plus the Tauri plugins the tree touches.
const useHostCaps = vi.fn();
vi.mock("@/hooks/useHostCaps", () => ({
  useHostCaps: () => useHostCaps(),
}));

vi.mock("@/hooks/useVmLibrary", () => ({
  useVmLibrary: () => ({ vms: [], loading: false, error: null, refresh: vi.fn() }),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

import App from "@/App";

function healthyCaps(over: Partial<HostCapabilities> = {}): HostCapabilities {
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

function hostCaps(over: Partial<UseHostCaps>): UseHostCaps {
  return {
    caps: null,
    loading: false,
    refreshing: false,
    error: null,
    refresh: vi.fn().mockResolvedValue(null),
    hostCores: 8,
    ...over,
  };
}

beforeEach(() => {
  useHostCaps.mockReset();
});

describe("App first-run gate wiring", () => {
  it("shows the loading screen while the initial probe is in flight", () => {
    useHostCaps.mockReturnValue(hostCaps({ caps: null, loading: true }));
    render(<App />);
    expect(screen.getByText(/Detecting host capabilities/i)).toBeInTheDocument();
    // No gate, no library yet.
    expect(screen.queryByText(/QEMU is required/i)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /new vm/i }),
    ).not.toBeInTheDocument();
  });

  it("shows the QEMU gate and NOT the library when QEMU is missing", () => {
    const missing = healthyCaps({
      qemu_img: { name: "qemu-img", present: false, version: null },
      system_binaries: [
        { name: "qemu-system-aarch64", present: false, version: null },
        { name: "qemu-system-x86_64", present: false, version: null },
      ],
    });
    useHostCaps.mockReturnValue(hostCaps({ caps: missing, loading: false }));

    render(<App />);
    expect(screen.getByText(/QEMU is required to run VMForge/i)).toBeInTheDocument();
    // The library's "New VM" button must NOT be present behind the gate.
    expect(
      screen.queryByRole("button", { name: /new vm/i }),
    ).not.toBeInTheDocument();
  });

  it("shows the library and NOT the gate when the host is healthy", () => {
    useHostCaps.mockReturnValue(
      hostCaps({ caps: healthyCaps(), loading: false }),
    );

    render(<App />);
    expect(
      screen.getByRole("button", { name: /new vm/i }),
    ).toBeInTheDocument();
    expect(screen.queryByText(/QEMU is required/i)).not.toBeInTheDocument();
  });
});
