import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { PortForwardRow } from "@/components/common/PortForwardRow";
import type { PortForward } from "@/lib/ipc";

function base(over: Partial<PortForward> = {}): PortForward {
  return { host: 2222, guest: 22, udp: false, expose_lan: false, ...over };
}

function renderRow(props: Partial<React.ComponentProps<typeof PortForwardRow>> = {}) {
  const onChange = vi.fn();
  const onRemove = vi.fn();
  render(
    <PortForwardRow
      value={base()}
      index={0}
      onChange={onChange}
      onRemove={onRemove}
      {...props}
    />,
  );
  return { onChange, onRemove };
}

describe("PortForwardRow", () => {
  it("renders host/guest values", () => {
    renderRow();
    expect(screen.getByLabelText(/host port for forward 1/i)).toHaveValue(2222);
    expect(screen.getByLabelText(/guest port for forward 1/i)).toHaveValue(22);
  });

  it("emits an updated host port on change", () => {
    const { onChange } = renderRow();
    fireEvent.change(screen.getByLabelText(/host port for forward 1/i), {
      target: { value: "8080" },
    });
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ host: 8080 }),
    );
  });

  it("toggles expose_lan", () => {
    const { onChange } = renderRow();
    fireEvent.click(screen.getByLabelText(/expose forward 1 to the lan/i));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ expose_lan: true }),
    );
  });

  it("calls onRemove when the remove button is clicked", () => {
    const { onRemove } = renderRow();
    fireEvent.click(screen.getByLabelText(/remove forward 1/i));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });

  it("shows an inline error and marks inputs invalid", () => {
    renderRow({ error: "Duplicate TCP host port 2222." });
    expect(screen.getByText(/duplicate tcp host port/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/host port for forward 1/i)).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("shows a soft warning when there is no error", () => {
    renderRow({ warning: "Host ports below 1024 usually need elevated privileges to bind." });
    expect(screen.getByText(/below 1024/i)).toBeInTheDocument();
  });

  it("prefers the error over the warning when both are present", () => {
    renderRow({ error: "bad", warning: "warn" });
    expect(screen.getByText("bad")).toBeInTheDocument();
    expect(screen.queryByText("warn")).not.toBeInTheDocument();
  });

  it("disables every control when disabled", () => {
    renderRow({ disabled: true });
    expect(screen.getByLabelText(/host port for forward 1/i)).toBeDisabled();
    expect(screen.getByLabelText(/guest port for forward 1/i)).toBeDisabled();
    expect(screen.getByLabelText(/expose forward 1 to the lan/i)).toBeDisabled();
    expect(screen.getByLabelText(/remove forward 1/i)).toBeDisabled();
  });
});
