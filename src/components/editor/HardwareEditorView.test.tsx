import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { VmConfig } from "@/lib/ipc";

// Mock the Tauri IPC + dialog plugin before importing the component.
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

import { HardwareEditorView, isEditable } from "@/components/editor/HardwareEditorView";

const CONFIG: VmConfig = {
  id: "vm-1",
  name: "Alpine",
  hardware: { cpus: 2, memory_mib: 2048 },
  disks: [{ path: "disk.qcow2", size_gib: 20, backing: null }],
  network: { mode: "user", mac: null, port_forwards: [] },
  iso: null,
};

function renderEditor(state: Parameters<typeof HardwareEditorView>[0]["state"]) {
  return render(
    <TooltipProvider>
      <HardwareEditorView
        vmId="vm-1"
        state={state}
        hostCores={8}
        onClose={() => {}}
        onSaved={() => {}}
      />
    </TooltipProvider>,
  );
}

beforeEach(() => {
  invoke.mockReset();
  // get_vm resolves the config; other commands resolve undefined.
  invoke.mockImplementation((cmd: string) => {
    if (cmd === "get_vm") return Promise.resolve(CONFIG);
    return Promise.resolve(undefined);
  });
});

describe("isEditable", () => {
  it("is true only for stopped/defined", () => {
    expect(isEditable("stopped")).toBe(true);
    expect(isEditable("defined")).toBe(true);
    expect(isEditable("running")).toBe(false);
    expect(isEditable("paused")).toBe(false);
    expect(isEditable("starting")).toBe(false);
    expect(isEditable("error")).toBe(false);
  });
});

describe("HardwareEditorView field gating", () => {
  it("enables the name field when the VM is stopped", async () => {
    renderEditor("stopped");
    const name = await screen.findByLabelText("Name");
    expect(name).not.toBeDisabled();
  });

  it("disables fields when the VM is running (state != stopped)", async () => {
    renderEditor("running");
    const name = await screen.findByLabelText("Name");
    expect(name).toBeDisabled();
    // A locked banner explains why editing is unavailable.
    expect(
      screen.getByText(/only be edited while the VM is stopped/i),
    ).toBeInTheDocument();
  });

  it("disables fields when the VM is paused", async () => {
    renderEditor("paused");
    const name = await screen.findByLabelText("Name");
    await waitFor(() => expect(name).toBeDisabled());
  });
});
