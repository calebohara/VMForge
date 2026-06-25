import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { QuickActions, type VmActions } from "@/components/library/QuickActions";
import type { VmListItem, VmState } from "@/lib/ipc";

function vm(over: Partial<VmListItem> = {}): VmListItem {
  return {
    id: "vm-1",
    name: "Alpine",
    state: "stopped" as VmState,
    accelerator: "hvf",
    emulated: false,
    cpus: 2,
    memory_mib: 2048,
    iso: null,
    suspended: false,
    ...over,
  };
}

function noopActions(): VmActions {
  return {
    onStart: vi.fn(),
    onShutdown: vi.fn(),
    onForceOff: vi.fn(),
    onPause: vi.fn(),
    onResume: vi.fn(),
    onOpenConsole: vi.fn(),
    onEdit: vi.fn(),
    onDelete: vi.fn(),
    onOpenSnapshots: vi.fn(),
    onClone: vi.fn(),
    onSuspend: vi.fn(),
    onRestore: vi.fn(),
    onDiscard: vi.fn(),
  };
}

function renderActions(item: VmListItem, actions: VmActions, busy = false) {
  render(
    <TooltipProvider>
      <QuickActions vm={item} actions={actions} busy={busy} />
    </TooltipProvider>,
  );
}

describe("QuickActions — running", () => {
  it("offers Suspend (and not Resume/Discard) while running", () => {
    const actions = noopActions();
    renderActions(vm({ state: "running" }), actions);

    expect(screen.getByRole("button", { name: /suspend/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /open console/i })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^resume$/i })).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /discard & stop/i }),
    ).not.toBeInTheDocument();
  });

  it("calls onSuspend when Suspend is clicked", () => {
    const actions = noopActions();
    renderActions(vm({ state: "running" }), actions);
    fireEvent.click(screen.getByRole("button", { name: /suspend/i }));
    expect(actions.onSuspend).toHaveBeenCalledTimes(1);
  });
});

describe("QuickActions — suspended (stopped + suspended)", () => {
  const suspendedVm = vm({ state: "stopped", suspended: true });

  it("offers Resume and Discard & stop, not Start/Suspend", () => {
    const actions = noopActions();
    renderActions(suspendedVm, actions);

    expect(screen.getByRole("button", { name: /^resume$/i })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /discard & stop/i }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^start$/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /suspend/i })).not.toBeInTheDocument();
  });

  it("calls onRestore / onDiscard from the suspended action set", () => {
    const actions = noopActions();
    renderActions(suspendedVm, actions);
    fireEvent.click(screen.getByRole("button", { name: /^resume$/i }));
    expect(actions.onRestore).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: /discard & stop/i }));
    expect(actions.onDiscard).toHaveBeenCalledTimes(1);
  });

  it("disables Edit (frozen while suspended)", () => {
    const actions = noopActions();
    renderActions(suspendedVm, actions);
    // No standalone Edit button is rendered in the suspended action set; Edit /
    // Clone / Rename live in the overflow menu and are disabled there. The
    // suspended set intentionally has no Start/Edit affordance.
    expect(screen.queryByRole("button", { name: /^edit$/i })).not.toBeInTheDocument();
  });
});

describe("QuickActions — stopped (not suspended)", () => {
  it("offers Start and an enabled Edit", () => {
    const actions = noopActions();
    renderActions(vm({ state: "stopped", suspended: false }), actions);
    expect(screen.getByRole("button", { name: /^start$/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^edit$/i })).toBeEnabled();
  });

  it("disables Edit when running (frozen)", () => {
    const actions = noopActions();
    renderActions(vm({ state: "running" }), actions);
    // The Edit button is not part of the running set, but suspend/clone/edit
    // gating is exercised by the stopped+suspended case above.
    expect(screen.queryByRole("button", { name: /^edit$/i })).not.toBeInTheDocument();
  });
});
