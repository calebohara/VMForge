import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DeleteVmDialog } from "@/components/library/DeleteVmDialog";
import type { VmListItem } from "@/lib/ipc";

const VM: VmListItem = {
  id: "vm-1",
  name: "Alpine",
  state: "stopped",
  accelerator: "hvf",
  emulated: false,
  cpus: 2,
  memory_mib: 2048,
  iso: null,
};

describe("DeleteVmDialog", () => {
  it("does not confirm until the user clicks Delete", () => {
    const onConfirm = vi.fn();
    render(<DeleteVmDialog vm={VM} onCancel={() => {}} onConfirm={onConfirm} />);
    // Opening the dialog alone must not trigger a delete.
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("confirms with the VM id and delete_disks flag when Delete is clicked", () => {
    const onConfirm = vi.fn();
    render(<DeleteVmDialog vm={VM} onCancel={() => {}} onConfirm={onConfirm} />);

    fireEvent.click(screen.getByRole("button", { name: /^delete$/i }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    // delete_disks defaults to true (checkbox checked on open).
    expect(onConfirm).toHaveBeenCalledWith("vm-1", true);
  });

  it("can opt out of deleting disks before confirming", () => {
    const onConfirm = vi.fn();
    render(<DeleteVmDialog vm={VM} onCancel={() => {}} onConfirm={onConfirm} />);

    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: /^delete$/i }));
    expect(onConfirm).toHaveBeenCalledWith("vm-1", false);
  });

  it("cancels without confirming", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(<DeleteVmDialog vm={VM} onCancel={onCancel} onConfirm={onConfirm} />);

    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(onConfirm).not.toHaveBeenCalled();
    expect(onCancel).toHaveBeenCalled();
  });

  it("renders nothing actionable when no VM is selected", () => {
    const onConfirm = vi.fn();
    render(
      <DeleteVmDialog vm={null} onCancel={() => {}} onConfirm={onConfirm} />,
    );
    expect(
      screen.queryByRole("button", { name: /^delete$/i }),
    ).not.toBeInTheDocument();
  });
});
