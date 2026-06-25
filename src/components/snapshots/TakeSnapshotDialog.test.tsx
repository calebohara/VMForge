import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { TakeSnapshotDialog } from "@/components/snapshots/TakeSnapshotDialog";

function renderDialog(
  props: Partial<React.ComponentProps<typeof TakeSnapshotDialog>> = {},
) {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(
    <TakeSnapshotDialog
      open
      live={false}
      busy={false}
      onCancel={onCancel}
      onConfirm={onConfirm}
      {...props}
    />,
  );
  return { onConfirm, onCancel };
}

describe("TakeSnapshotDialog", () => {
  it("disables 'Take snapshot' until a name is entered", () => {
    renderDialog();
    expect(
      screen.getByRole("button", { name: /take snapshot/i }),
    ).toBeDisabled();
  });

  it("confirms with the trimmed name", () => {
    const { onConfirm } = renderDialog();
    fireEvent.change(screen.getByLabelText(/snapshot name/i), {
      target: { value: "  Before update  " },
    });
    fireEvent.click(screen.getByRole("button", { name: /take snapshot/i }));
    expect(onConfirm).toHaveBeenCalledWith("Before update");
  });

  it("rejects an invalid name (path separators)", () => {
    renderDialog();
    fireEvent.change(screen.getByLabelText(/snapshot name/i), {
      target: { value: "bad/name" },
    });
    expect(
      screen.getByRole("button", { name: /take snapshot/i }),
    ).toBeDisabled();
  });

  it("mentions memory capture when the source is live", () => {
    renderDialog({ live: true });
    expect(screen.getByText(/disk and memory state/i)).toBeInTheDocument();
  });

  it("shows the 'Saving memory state' spinner while a live take is in flight", () => {
    renderDialog({ live: true, busy: true });
    expect(screen.getByText(/saving memory state/i)).toBeInTheDocument();
    // Cancel is disabled mid-flight.
    expect(screen.getByRole("button", { name: /cancel/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /taking/i })).toBeDisabled();
  });

  it("does not confirm just by opening", () => {
    const { onConfirm } = renderDialog();
    expect(onConfirm).not.toHaveBeenCalled();
  });
});
