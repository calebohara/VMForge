import { describe, it, expect } from "vitest";
import {
  qemuMissing,
  hasHostWarnings,
  nativeSystemBinaryName,
  requiredBinaries,
  installGuide,
} from "@/lib/hostStatus";
import type { HostCapabilities, QemuBinary } from "@/lib/ipc";

function bin(name: string, present: boolean, version: string | null = null): QemuBinary {
  return { name, present, version };
}

function caps(over: Partial<HostCapabilities> = {}): HostCapabilities {
  return {
    os: "macos",
    arch: "aarch64",
    preferred_accelerator: "hvf",
    available_accelerators: ["hvf", "tcg"],
    hardware_accelerated: true,
    qemu_img: bin("qemu-img", true, "qemu-img version 11.0.1"),
    system_binaries: [
      bin("qemu-system-aarch64", true, "QEMU emulator version 11.0.1"),
      bin("qemu-system-x86_64", true, "QEMU emulator version 11.0.1"),
    ],
    network: { modes: [], port_forward_loopback_only: true },
    warnings: [],
    ...over,
  };
}

describe("qemuMissing", () => {
  it("is false for a fully healthy host", () => {
    expect(qemuMissing(caps())).toBe(false);
  });

  it("is false when caps are still null (loading, not yet probed)", () => {
    expect(qemuMissing(null)).toBe(false);
  });

  it("is true when no qemu-system-* binary is present", () => {
    expect(
      qemuMissing(
        caps({
          system_binaries: [
            bin("qemu-system-aarch64", false),
            bin("qemu-system-x86_64", false),
          ],
        }),
      ),
    ).toBe(true);
  });

  it("is true when qemu-img is absent even if a system binary exists", () => {
    expect(qemuMissing(caps({ qemu_img: bin("qemu-img", false) }))).toBe(true);
  });

  it("is false when only one of the two system binaries is present", () => {
    expect(
      qemuMissing(
        caps({
          system_binaries: [
            bin("qemu-system-aarch64", true, "11.0.1"),
            bin("qemu-system-x86_64", false),
          ],
        }),
      ),
    ).toBe(false);
  });
});

describe("hasHostWarnings", () => {
  it("is false for a clean, accelerated host with no warnings", () => {
    expect(hasHostWarnings(caps())).toBe(false);
  });

  it("is false for null caps", () => {
    expect(hasHostWarnings(null)).toBe(false);
  });

  it("is true when the probe emitted warnings", () => {
    expect(hasHostWarnings(caps({ warnings: ["Apple Silicon note"] }))).toBe(true);
  });

  it("is true when hardware acceleration is unavailable (TCG)", () => {
    expect(
      hasHostWarnings(
        caps({ hardware_accelerated: false, preferred_accelerator: "tcg" }),
      ),
    ).toBe(true);
  });
});

describe("nativeSystemBinaryName", () => {
  it("maps aarch64/arm64 to qemu-system-aarch64", () => {
    expect(nativeSystemBinaryName("aarch64")).toBe("qemu-system-aarch64");
    expect(nativeSystemBinaryName("arm64")).toBe("qemu-system-aarch64");
  });

  it("maps x86_64/x64/amd64 to qemu-system-x86_64", () => {
    expect(nativeSystemBinaryName("x86_64")).toBe("qemu-system-x86_64");
    expect(nativeSystemBinaryName("x64")).toBe("qemu-system-x86_64");
    expect(nativeSystemBinaryName("amd64")).toBe("qemu-system-x86_64");
  });

  it("falls back to qemu-system-x86_64 for unknown arches", () => {
    expect(nativeSystemBinaryName("riscv64")).toBe("qemu-system-x86_64");
  });
});

describe("requiredBinaries", () => {
  it("lists qemu-img then the host-native qemu-system, carrying probed state", () => {
    const list = requiredBinaries(caps());
    expect(list.map((b) => b.name)).toEqual([
      "qemu-img",
      "qemu-system-aarch64",
    ]);
    expect(list[0].present).toBe(true);
    expect(list[1].version).toBe("QEMU emulator version 11.0.1");
  });

  it("synthesizes an absent record when the probe omitted the native binary", () => {
    const list = requiredBinaries(
      caps({ arch: "aarch64", system_binaries: [] }),
    );
    expect(list[1]).toEqual({
      name: "qemu-system-aarch64",
      present: false,
      version: null,
    });
  });
});

describe("installGuide", () => {
  it("returns a Homebrew command for macOS", () => {
    const g = installGuide("macos");
    expect(g.heading).toMatch(/Homebrew/i);
    expect(g.steps.some((s) => s.command === "brew install qemu")).toBe(true);
    expect(g.docsUrl).toContain("qemu.org");
  });

  it("covers Debian, Fedora, and Arch for Linux", () => {
    const g = installGuide("linux");
    const commands = g.steps.map((s) => s.command ?? "").join("\n");
    expect(commands).toMatch(/apt install/);
    expect(commands).toMatch(/dnf install/);
    expect(commands).toMatch(/pacman -S/);
  });

  it("gives Windows installer + PATH guidance", () => {
    const g = installGuide("windows");
    const text = g.steps.map((s) => s.description).join(" ");
    expect(text).toMatch(/installer/i);
    expect(text).toMatch(/PATH/);
  });

  it("falls back to a generic guide for unknown OSes", () => {
    const g = installGuide("plan9");
    expect(g.docsUrl).toContain("qemu.org");
    expect(g.steps.length).toBeGreaterThan(0);
  });
});
