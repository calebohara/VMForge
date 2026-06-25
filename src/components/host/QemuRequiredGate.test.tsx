import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { HostCapabilities } from "@/lib/ipc";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

const dialogOpen = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => dialogOpen(...args),
}));

const openUrl = vi.fn();
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: (...args: unknown[]) => openUrl(...args),
}));

import { QemuRequiredGate } from "@/components/host/QemuRequiredGate";

function caps(over: Partial<HostCapabilities> = {}): HostCapabilities {
  return {
    os: "macos",
    arch: "aarch64",
    preferred_accelerator: "tcg",
    available_accelerators: ["tcg"],
    hardware_accelerated: false,
    qemu_img: { name: "qemu-img", present: false, version: null },
    system_binaries: [
      { name: "qemu-system-aarch64", present: false, version: null },
      { name: "qemu-system-x86_64", present: false, version: null },
    ],
    network: { modes: [], port_forward_loopback_only: true },
    warnings: [],
    ...over,
  };
}

beforeEach(() => {
  invoke.mockReset();
  dialogOpen.mockReset();
  openUrl.mockReset();
});

describe("QemuRequiredGate", () => {
  it("names the missing binaries and shows versions of found ones", () => {
    const c = caps({
      qemu_img: { name: "qemu-img", present: true, version: "qemu-img 11.0.1" },
    });
    render(<QemuRequiredGate caps={c} onRecheck={vi.fn().mockResolvedValue(c)} />);

    // The native system binary is missing and named explicitly.
    expect(screen.getByText("qemu-system-aarch64")).toBeInTheDocument();
    // qemu-img was found; its version is surfaced.
    expect(screen.getByText("qemu-img 11.0.1")).toBeInTheDocument();
    // honest "not found" marker for the missing one.
    expect(screen.getAllByText(/not found/i).length).toBeGreaterThan(0);
  });

  it("re-checks via onRecheck (re-probe, no restart)", () => {
    const onRecheck = vi.fn().mockResolvedValue(caps());
    render(<QemuRequiredGate caps={caps()} onRecheck={onRecheck} />);

    fireEvent.click(screen.getByRole("button", { name: /re-check/i }));
    expect(onRecheck).toHaveBeenCalledTimes(1);
  });

  it("disables buttons and spins while rechecking", () => {
    render(
      <QemuRequiredGate
        caps={caps()}
        rechecking
        onRecheck={vi.fn().mockResolvedValue(caps())}
      />,
    );
    expect(screen.getByRole("button", { name: /re-check/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /locate qemu/i })).toBeDisabled();
  });

  it("Locate QEMU… persists the picked dir then re-probes", async () => {
    dialogOpen.mockResolvedValue("/opt/homebrew/bin");
    invoke.mockResolvedValue(undefined); // set_qemu_dir
    const onRecheck = vi.fn().mockResolvedValue(caps());

    render(<QemuRequiredGate caps={caps()} onRecheck={onRecheck} />);
    fireEvent.click(screen.getByRole("button", { name: /locate qemu/i }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("set_qemu_dir", {
        dir: "/opt/homebrew/bin",
      }),
    );
    await waitFor(() => expect(onRecheck).toHaveBeenCalledTimes(1));
    expect(dialogOpen).toHaveBeenCalledWith({ directory: true, multiple: false });
  });

  it("does nothing when the directory picker is cancelled", async () => {
    dialogOpen.mockResolvedValue(null); // user cancelled
    const onRecheck = vi.fn().mockResolvedValue(caps());

    render(<QemuRequiredGate caps={caps()} onRecheck={onRecheck} />);
    fireEvent.click(screen.getByRole("button", { name: /locate qemu/i }));

    await waitFor(() => expect(dialogOpen).toHaveBeenCalled());
    expect(invoke).not.toHaveBeenCalled();
    expect(onRecheck).not.toHaveBeenCalled();
  });

  it("opens the docs link via the opener plugin", async () => {
    openUrl.mockResolvedValue(undefined);
    render(<QemuRequiredGate caps={caps()} onRecheck={vi.fn().mockResolvedValue(caps())} />);

    fireEvent.click(screen.getByRole("button", { name: /download page/i }));
    await waitFor(() => expect(openUrl).toHaveBeenCalledTimes(1));
    expect(openUrl).toHaveBeenCalledWith(expect.stringContaining("qemu.org"));
  });

  it("surfaces a probe error when one is supplied", () => {
    render(
      <QemuRequiredGate
        caps={caps()}
        error="probe blew up"
        onRecheck={vi.fn().mockResolvedValue(caps())}
      />,
    );
    expect(screen.getByText(/probe blew up/)).toBeInTheDocument();
  });
});
