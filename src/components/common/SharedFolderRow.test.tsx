import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { SharedFolderRow } from "@/components/common/SharedFolderRow";
import type { SharedFolder } from "@/lib/ipc";

// DirectoryPicker imports the Tauri dialog plugin; mock it so the Browse button
// doesn't reach for the native dialog under jsdom.
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

function base(over: Partial<SharedFolder> = {}): SharedFolder {
  return { host_path: "/host/dir", mount_tag: "shared", read_only: false, ...over };
}

function renderRow(
  props: Partial<React.ComponentProps<typeof SharedFolderRow>> = {},
) {
  const onChange = vi.fn();
  const onRemove = vi.fn();
  render(
    <SharedFolderRow
      value={base()}
      index={0}
      onChange={onChange}
      onRemove={onRemove}
      {...props}
    />,
  );
  return { onChange, onRemove };
}

describe("SharedFolderRow", () => {
  it("renders the host path and mount tag", () => {
    renderRow();
    expect(screen.getByLabelText(/host folder for share 1/i)).toHaveValue(
      "/host/dir",
    );
    expect(screen.getByLabelText(/mount tag for share 1/i)).toHaveValue(
      "shared",
    );
  });

  it("emits an updated mount tag on change", () => {
    const { onChange } = renderRow();
    fireEvent.change(screen.getByLabelText(/mount tag for share 1/i), {
      target: { value: "docs" },
    });
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ mount_tag: "docs" }),
    );
  });

  it("emits an updated host path on change", () => {
    const { onChange } = renderRow();
    fireEvent.change(screen.getByLabelText(/host folder for share 1/i), {
      target: { value: "/new/path" },
    });
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ host_path: "/new/path" }),
    );
  });

  it("toggles read_only", () => {
    const { onChange } = renderRow();
    fireEvent.click(screen.getByLabelText(/make share 1 read-only/i));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ read_only: true }),
    );
  });

  it("calls onRemove when the remove button is clicked", () => {
    const { onRemove } = renderRow();
    fireEvent.click(screen.getByLabelText(/remove share 1/i));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });

  it("shows an inline error and marks the tag invalid", () => {
    renderRow({ error: "Duplicate mount tag “shared”." });
    expect(screen.getByText(/duplicate mount tag/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/mount tag for share 1/i)).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("disables every control when disabled", () => {
    renderRow({ disabled: true });
    expect(screen.getByLabelText(/host folder for share 1/i)).toBeDisabled();
    expect(screen.getByLabelText(/mount tag for share 1/i)).toBeDisabled();
    expect(screen.getByLabelText(/make share 1 read-only/i)).toBeDisabled();
    expect(screen.getByLabelText(/remove share 1/i)).toBeDisabled();
  });
});
