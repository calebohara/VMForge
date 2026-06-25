import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock the Tauri invoke before importing the module under test.
const invoke = vi.fn((..._args: unknown[]) => Promise.resolve(undefined as unknown));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

import * as ipc from "@/lib/ipc";
import type { CreateVmRequest, UpdateVmRequest } from "@/lib/ipc";

beforeEach(() => {
  invoke.mockClear();
});

describe("ipc wrappers call invoke with the exact command + arg shape", () => {
  it("probeHost", () => {
    ipc.probeHost();
    expect(invoke).toHaveBeenCalledWith("probe_host");
  });

  it("createVm passes { req } with snake_case body", () => {
    const req: CreateVmRequest = {
      name: "Alpine",
      hardware: { cpus: 2, memory_mib: 2048 },
      disk_gib: 8,
      network: null,
      iso: "/x.iso",
    };
    ipc.createVm(req);
    expect(invoke).toHaveBeenCalledWith("create_vm", { req });
  });

  it("listVms takes no args", () => {
    ipc.listVms();
    expect(invoke).toHaveBeenCalledWith("list_vms");
  });

  it("getVm passes { id }", () => {
    ipc.getVm("abc");
    expect(invoke).toHaveBeenCalledWith("get_vm", { id: "abc" });
  });

  it("updateVm passes { id, req }", () => {
    const req: UpdateVmRequest = {
      name: "New",
      hardware: { cpus: 4, memory_mib: 4096 },
      network: null,
      iso: null,
    };
    ipc.updateVm("abc", req);
    expect(invoke).toHaveBeenCalledWith("update_vm", { id: "abc", req });
  });

  it("deleteVm passes snake_case { id, delete_disks }", () => {
    ipc.deleteVm("abc", true);
    expect(invoke).toHaveBeenCalledWith("delete_vm", {
      id: "abc",
      delete_disks: true,
    });
  });

  it("startVm passes { id }", () => {
    ipc.startVm("abc");
    expect(invoke).toHaveBeenCalledWith("start_vm", { id: "abc" });
  });

  it("renameVm passes snake_case { id, new_name }", () => {
    ipc.renameVm("abc", "Renamed");
    expect(invoke).toHaveBeenCalledWith("rename_vm", {
      id: "abc",
      new_name: "Renamed",
    });
  });

  it("openConsole passes { id }", () => {
    ipc.openConsole("abc");
    expect(invoke).toHaveBeenCalledWith("open_console", { id: "abc" });
  });

  it("vmState passes { id }", () => {
    ipc.vmState("abc");
    expect(invoke).toHaveBeenCalledWith("vm_state", { id: "abc" });
  });

  it("powerOff / forceOff / pauseVm / resumeVm pass { id }", () => {
    ipc.powerOff("a");
    expect(invoke).toHaveBeenLastCalledWith("power_off", { id: "a" });
    ipc.forceOff("a");
    expect(invoke).toHaveBeenLastCalledWith("force_off", { id: "a" });
    ipc.pauseVm("a");
    expect(invoke).toHaveBeenLastCalledWith("pause_vm", { id: "a" });
    ipc.resumeVm("a");
    expect(invoke).toHaveBeenLastCalledWith("resume_vm", { id: "a" });
  });

  // ---- Phase 5 suspend / restore wrappers ----

  it("suspendVm invokes suspend_vm (NOT pause_vm) with { id }", () => {
    ipc.suspendVm("a");
    expect(invoke).toHaveBeenLastCalledWith("suspend_vm", { id: "a" });
  });

  it("restoreVm invokes restore_vm (NOT resume_vm) with { id }", () => {
    ipc.restoreVm("a");
    expect(invoke).toHaveBeenLastCalledWith("restore_vm", { id: "a" });
  });

  it("discardSuspend invokes discard_suspend with { id }", () => {
    ipc.discardSuspend("a");
    expect(invoke).toHaveBeenLastCalledWith("discard_suspend", { id: "a" });
  });

  it("resumeVm and restoreVm are distinct commands", () => {
    ipc.resumeVm("a");
    expect(invoke).toHaveBeenLastCalledWith("resume_vm", { id: "a" });
    ipc.restoreVm("a");
    expect(invoke).toHaveBeenLastCalledWith("restore_vm", { id: "a" });
  });

  // ---- Phase 3 snapshot / clone wrappers ----

  it("listSnapshots passes { id }", () => {
    ipc.listSnapshots("abc");
    expect(invoke).toHaveBeenCalledWith("list_snapshots", { id: "abc" });
  });

  it("createSnapshot passes { id, name }", () => {
    ipc.createSnapshot("abc", "Before update");
    expect(invoke).toHaveBeenCalledWith("create_snapshot", {
      id: "abc",
      name: "Before update",
    });
  });

  it("restoreSnapshot passes snake_case { id, snapshot_id }", () => {
    ipc.restoreSnapshot("abc", "snap-1");
    expect(invoke).toHaveBeenCalledWith("restore_snapshot", {
      id: "abc",
      snapshot_id: "snap-1",
    });
  });

  it("deleteSnapshot passes snake_case { id, snapshot_id }", () => {
    ipc.deleteSnapshot("abc", "snap-1");
    expect(invoke).toHaveBeenCalledWith("delete_snapshot", {
      id: "abc",
      snapshot_id: "snap-1",
    });
  });

  it("cloneVm passes snake_case { id, new_name, linked }", () => {
    ipc.cloneVm("abc", "Clone", true);
    expect(invoke).toHaveBeenCalledWith("clone_vm", {
      id: "abc",
      new_name: "Clone",
      linked: true,
    });
  });
});
