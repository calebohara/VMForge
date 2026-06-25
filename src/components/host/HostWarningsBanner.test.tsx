import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { HostWarningsBanner } from "@/components/host/HostWarningsBanner";
import type { HostCapabilities } from "@/lib/ipc";

function caps(over: Partial<HostCapabilities> = {}): HostCapabilities {
  return {
    os: "macos",
    arch: "aarch64",
    preferred_accelerator: "hvf",
    available_accelerators: ["hvf", "tcg"],
    hardware_accelerated: true,
    qemu_img: { name: "qemu-img", present: true, version: "11.0.1" },
    system_binaries: [
      { name: "qemu-system-aarch64", present: true, version: "11.0.1" },
    ],
    network: { modes: [], port_forward_loopback_only: true },
    warnings: [],
    ...over,
  };
}

describe("HostWarningsBanner", () => {
  it("renders nothing for a clean, accelerated host", () => {
    const { container } = render(<HostWarningsBanner caps={caps()} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("renders nothing for null caps", () => {
    const { container } = render(<HostWarningsBanner caps={null} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("renders each probe warning", () => {
    render(
      <HostWarningsBanner
        caps={caps({
          warnings: ["Apple Silicon: x86 guests run under TCG.", "Second note."],
        })}
      />,
    );
    expect(
      screen.getByText(/Apple Silicon: x86 guests run under TCG\./),
    ).toBeInTheDocument();
    expect(screen.getByText(/Second note\./)).toBeInTheDocument();
  });

  it("shows a fallback message when acceleration is unavailable but no warning string was emitted", () => {
    render(
      <HostWarningsBanner
        caps={caps({ hardware_accelerated: false, warnings: [] })}
      />,
    );
    expect(screen.getByText(/TCG software emulation/i)).toBeInTheDocument();
  });
});
