import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { CloneVmDialog } from "@/components/snapshots/CloneVmDialog";

function renderDialog(
  props: Partial<React.ComponentProps<typeof CloneVmDialog>> = {},
) {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(
    <CloneVmDialog
      open
      sourceName="Alpine"
      busy={false}
      onCancel={onCancel}
      onConfirm={onConfirm}
      {...props}
    />,
  );
  return { onConfirm, onCancel };
}

describe("CloneVmDialog", () => {
  it("defaults the name to '<source> clone' and the type to full", () => {
    renderDialog();
    expect(screen.getByLabelText("New VM name")).toHaveValue("Alpine clone");
    // Full is selected by default; the linked-consequence callout is hidden.
    expect(
      screen.queryByText(/linked clone depends on/i),
    ).not.toBeInTheDocument();
  });

  it("confirms with the name and linked=false for a full clone", () => {
    const { onConfirm } = renderDialog();
    fireEvent.click(screen.getByRole("button", { name: /^clone$/i }));
    expect(onConfirm).toHaveBeenCalledWith("Alpine clone", false);
  });

  it("shows the amber consequence callout and confirms linked=true when Linked is chosen", () => {
    const { onConfirm } = renderDialog();
    fireEvent.click(screen.getByLabelText(/linked clone/i));
    expect(screen.getByText(/linked clone depends on/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^clone$/i }));
    expect(onConfirm).toHaveBeenCalledWith("Alpine clone", true);
  });

  it("disables Clone when the name is empty", () => {
    renderDialog();
    fireEvent.change(screen.getByLabelText("New VM name"), {
      target: { value: "   " },
    });
    expect(screen.getByRole("button", { name: /^clone$/i })).toBeDisabled();
  });

  it("shows 'Cloning…' and disables the footer while busy", () => {
    renderDialog({ busy: true });
    expect(
      screen.getByRole("button", { name: /cloning/i }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: /cancel/i })).toBeDisabled();
  });

  it("does not confirm just by opening", () => {
    const { onConfirm } = renderDialog();
    expect(onConfirm).not.toHaveBeenCalled();
  });
});
