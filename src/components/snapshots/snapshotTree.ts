// Pure snapshot-tree assembly. No React, no IPC — directly unit-testable.
//
// The qcow2 internal snapshot list is flat; the VMware-style tree is OUR overlay
// built from each Snapshot's `parent_id`. This module turns the flat DTO array
// into a forest of {@link TreeNode}s with these contracts (spec §A1, §D3):
//
//   - Flat fallback: when *every* snapshot has `parent_id === null` the result
//     is a single-level list of roots in input order.
//   - Cycle-safe: a parent chain that loops back on itself never recurses
//     infinitely; a node whose ancestry forms a cycle is promoted to a root.
//   - Orphans -> roots: a `parent_id` that doesn't resolve to a known snapshot
//     is treated as no-parent (the node becomes a root).
import type { Snapshot } from "@/lib/ipc";

export interface TreeNode {
  snapshot: Snapshot;
  children: TreeNode[];
  /** Depth from its root (0 for roots). Handy for indentation. */
  depth: number;
}

/**
 * Build a forest of snapshot nodes from the flat DTO list. Roots preserve input
 * order; children preserve input order within each parent.
 */
export function buildTree(snapshots: Snapshot[]): TreeNode[] {
  // Index by id, building empty nodes first so ordering is stable.
  const byId = new Map<string, TreeNode>();
  for (const s of snapshots) {
    byId.set(s.snapshot_id, { snapshot: s, children: [], depth: 0 });
  }

  const roots: TreeNode[] = [];

  for (const s of snapshots) {
    const node = byId.get(s.snapshot_id)!;
    const parentId = s.parent_id;
    // No parent, unknown parent (orphan), or self-reference -> root.
    if (
      parentId == null ||
      parentId === s.snapshot_id ||
      !byId.has(parentId) ||
      formsCycle(s.snapshot_id, parentId, byId)
    ) {
      roots.push(node);
    } else {
      byId.get(parentId)!.children.push(node);
    }
  }

  // Assign depths via BFS from roots (only reachable nodes get touched, which is
  // exactly the forest we just built).
  for (const root of roots) assignDepths(root, 0);

  return roots;
}

/**
 * Collect the ids of every descendant of `snapshotId` (children, grandchildren,
 * …), NOT including the node itself. Cycle-safe via a visited set. Used to warn
 * that deleting a snapshot affects its subtree.
 */
export function descendantIds(
  snapshotId: string,
  snapshots: Snapshot[],
): string[] {
  // Build a child-adjacency map honoring the same orphan/self rules as buildTree.
  const ids = new Set(snapshots.map((s) => s.snapshot_id));
  const childrenOf = new Map<string, string[]>();
  for (const s of snapshots) {
    const p = s.parent_id;
    if (p != null && p !== s.snapshot_id && ids.has(p)) {
      const arr = childrenOf.get(p) ?? [];
      arr.push(s.snapshot_id);
      childrenOf.set(p, arr);
    }
  }

  const out: string[] = [];
  const seen = new Set<string>([snapshotId]);
  const stack = [...(childrenOf.get(snapshotId) ?? [])];
  while (stack.length > 0) {
    const id = stack.pop()!;
    if (seen.has(id)) continue;
    seen.add(id);
    out.push(id);
    for (const child of childrenOf.get(id) ?? []) {
      if (!seen.has(child)) stack.push(child);
    }
  }
  return out;
}

/**
 * Walk the parent chain from `parentId` upward; if we reach `selfId` before the
 * chain terminates we have a cycle. Bounded by the visited set.
 */
function formsCycle(
  selfId: string,
  parentId: string,
  byId: Map<string, TreeNode>,
): boolean {
  let current: string | null = parentId;
  const seen = new Set<string>([selfId]);
  while (current != null) {
    if (current === selfId) return true;
    if (seen.has(current)) return true;
    seen.add(current);
    const next: string | null | undefined = byId.get(current)?.snapshot.parent_id;
    current = next ?? null;
  }
  return false;
}

function assignDepths(node: TreeNode, depth: number): void {
  node.depth = depth;
  for (const child of node.children) assignDepths(child, depth + 1);
}
