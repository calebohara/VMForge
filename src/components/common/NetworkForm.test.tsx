import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { NetworkCapabilities, NetworkConfig } from "@/lib/ipc";

// Mock the Tauri invoke before importing the form (it probes capabilities).
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

import { NetworkForm } from "@/components/common/NetworkForm";
import { __resetNetworkCapsCache } from "@/hooks/useNetworkCaps";

const CAPS: NetworkCapabilities = {
  modes: [
    { mode: "user", available: true, requires_elevation: false, reason: "" },
    {
      mode: "bridged",
      available: false,
      requires_elevation: true,
      reason: "Bridged networking requires the macOS vmnet entitlement.",
    },
    {
      mode: "host-only",
      available: false,
      requires_elevation: true,
      reason: "Host-only networking requires the macOS vmnet entitlement.",
    },
  ],
  port_forward_loopback_only: true,
};

function userConfig(over: Partial<NetworkConfig> = {}): NetworkConfig {
  return { mode: "user", mac: null, port_forwards: [], ...over };
}

async function renderForm(
  props: Partial<React.ComponentProps<typeof NetworkForm>> = {},
) {
  const onChange = vi.fn();
  const onValidityChange = vi.fn();
  render(
    <NetworkForm
      value={userConfig()}
      onChange={onChange}
      onValidityChange={onValidityChange}
      {...props}
    />,
  );
  // Let the capability probe resolve.
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("network_capabilities"));
  return { onChange, onValidityChange };
}

beforeEach(() => {
  __resetNetworkCapsCache();
  invoke.mockReset();
  invoke.mockResolvedValue(CAPS);
});

describe("NetworkForm — mode capabilities", () => {
  it("probes capabilities once and shows the loopback security line in user mode", async () => {
    await renderForm();
    expect(
      await screen.findByText(/bind 127\.0\.0\.1 \(loopback\)/i),
    ).toBeInTheDocument();
  });

  it("does NOT auto-rewrite a legacy bridged value and shows an amber reason", async () => {
    const { onChange } = await renderForm({
      value: userConfig({ mode: "bridged" }),
    });
    // The legacy mode is shown selected; the form never calls onChange to fix it.
    expect(onChange).not.toHaveBeenCalled();
    expect(
      await screen.findByText(/requires elevated permissions and is not available/i),
    ).toBeInTheDocument();
    // The reason from capabilities is surfaced.
    expect(screen.getByText(/vmnet entitlement/i)).toBeInTheDocument();
  });

  it("hides the port-forward table outside user mode and explains why", async () => {
    await renderForm({ value: userConfig({ mode: "host-only" }) });
    expect(
      await screen.findByText(/port forwarding applies to nat \(user mode\) only/i),
    ).toBeInTheDocument();
  });
});

describe("NetworkForm — validity gating", () => {
  it("reports valid for an empty user-mode config", async () => {
    const { onValidityChange } = await renderForm();
    await waitFor(() =>
      expect(onValidityChange).toHaveBeenLastCalledWith(true),
    );
  });

  it("reports VALID for a legacy privileged mode (config persists; launch is refused, not save)", async () => {
    // Save/Create validity must not depend on mode availability: a
    // bridged/host-only config legitimately persists (A3); the engine refuses
    // only the launch. So an otherwise-clean bridged config is save-valid.
    const { onValidityChange } = await renderForm({
      value: userConfig({ mode: "bridged" }),
    });
    await waitFor(() =>
      expect(onValidityChange).toHaveBeenLastCalledWith(true),
    );
  });

  it("reports invalid when a MAC is malformed", async () => {
    const { onValidityChange } = await renderForm({
      value: userConfig({ mac: "not-a-mac" }),
    });
    await waitFor(() =>
      expect(onValidityChange).toHaveBeenLastCalledWith(false),
    );
    expect(
      screen.getByText(/six colon-separated hex octets/i),
    ).toBeInTheDocument();
  });

  it("reports invalid when a port forward is out of range", async () => {
    const { onValidityChange } = await renderForm({
      value: userConfig({
        port_forwards: [{ host: 0, guest: 22, udp: false, expose_lan: false }],
      }),
    });
    await waitFor(() =>
      expect(onValidityChange).toHaveBeenLastCalledWith(false),
    );
  });
});

describe("NetworkForm — MAC controls", () => {
  it("Generate populates a 52:54:00 MAC and Clear empties it", async () => {
    const { onChange } = await renderForm();
    fireEvent.click(screen.getByRole("button", { name: /generate/i }));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        mac: expect.stringMatching(/^52:54:00:/),
      }),
    );
  });
});

describe("NetworkForm — port forwards", () => {
  it("Add forward appends a row via onChange", async () => {
    const { onChange } = await renderForm();
    fireEvent.click(screen.getByRole("button", { name: /add forward/i }));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        port_forwards: [
          expect.objectContaining({ udp: false, expose_lan: false }),
        ],
      }),
    );
  });
});
