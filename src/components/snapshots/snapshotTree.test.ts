import { describe, it, expect } from "vitest";
import {
  buildTree,
  descendantIds,
  type TreeNode,
} from "@/components/snapshots/snapshotTree";
import type { Snapshot } from "@/lib/ipc";

function snap(id: string, parent: string | null = null): Snapshot {
  return {
    snapshot_id: id,
    name: id,
    parent_id: parent,
    created_at: "2026-06-24T00:00:00Z",
    has_vm_state: false,
    vm_state_size: 0,
    present_in_qcow2: true,
  };
}

/** Flatten a forest to "id@depth" strings (pre-order) for easy assertions. */
function flatten(nodes: TreeNode[]): string[] {
  const out: string[] = [];
  const walk = (n: TreeNode) => {
    out.push(`${n.snapshot.snapshot_id}@${n.depth}`);
    n.children.forEach(walk);
  };
  nodes.forEach(walk);
  return out;
}

describe("buildTree", () => {
  it("returns an empty forest for no snapshots", () => {
    expect(buildTree([])).toEqual([]);
  });

  it("degrades to a flat list when every parent_id is null", () => {
    const tree = buildTree([snap("a"), snap("b"), snap("c")]);
    expect(tree).toHaveLength(3);
    expect(flatten(tree)).toEqual(["a@0", "b@0", "c@0"]);
  });

  it("assembles a parent/child hierarchy with depths", () => {
    const tree = buildTree([
      snap("root"),
      snap("child", "root"),
      snap("grandchild", "child"),
    ]);
    expect(flatten(tree)).toEqual(["root@0", "child@1", "grandchild@2"]);
  });

  it("preserves input order for roots and siblings", () => {
    const tree = buildTree([
      snap("r1"),
      snap("r2"),
      snap("r1-b", "r1"),
      snap("r1-a", "r1"),
    ]);
    expect(flatten(tree)).toEqual(["r1@0", "r1-b@1", "r1-a@1", "r2@0"]);
  });

  it("promotes orphans (unknown parent) to roots", () => {
    const tree = buildTree([snap("a"), snap("b", "missing")]);
    expect(flatten(tree)).toEqual(["a@0", "b@0"]);
  });

  it("is cycle-safe: a self-referencing node becomes a root", () => {
    const tree = buildTree([snap("loop", "loop")]);
    expect(flatten(tree)).toEqual(["loop@0"]);
  });

  it("is cycle-safe: a two-node cycle does not recurse infinitely", () => {
    // a -> b -> a. Both reference each other; neither has a true root.
    const tree = buildTree([snap("a", "b"), snap("b", "a")]);
    // The forest contains both nodes (promoted to roots to break the cycle),
    // and the call terminates.
    const ids = flatten(tree)
      .map((s) => s.split("@")[0])
      .sort();
    expect(ids).toEqual(["a", "b"]);
  });
});

describe("descendantIds", () => {
  const snaps = [
    snap("root"),
    snap("a", "root"),
    snap("b", "root"),
    snap("a1", "a"),
    snap("a2", "a"),
    snap("a1x", "a1"),
  ];

  it("collects all descendants (transitively), excluding the node itself", () => {
    expect(descendantIds("root", snaps).sort()).toEqual(
      ["a", "a1", "a1x", "a2", "b"].sort(),
    );
    expect(descendantIds("a", snaps).sort()).toEqual(
      ["a1", "a1x", "a2"].sort(),
    );
  });

  it("returns an empty array for a leaf", () => {
    expect(descendantIds("b", snaps)).toEqual([]);
    expect(descendantIds("a1x", snaps)).toEqual([]);
  });

  it("is cycle-safe", () => {
    const cyclic = [snap("a", "b"), snap("b", "a")];
    // descendants of "a" via the a<-b adjacency; terminates without looping.
    expect(() => descendantIds("a", cyclic)).not.toThrow();
  });
});
