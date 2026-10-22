<script setup lang="ts">
import { computed } from "vue";
import type { AncestorView, ProcessView } from "../types";

const props = defineProps<{
  ancestry: AncestorView[];
  processes: ProcessView[];
  rootPid: number;
  confidence: string;
}>();

interface TreeRow {
  key: string;
  depth: number;
  pid: number;
  name: string;
  role: string;
  command: string;
  alive: boolean | null;
}

const sourceRows = computed<TreeRow[]>(() =>
  [...props.ancestry]
    .reverse()
    .map((entry, index) => ({
      key: `a-${entry.pid}-${entry.start_ticks}`,
      depth: index,
      pid: entry.pid,
      name: entry.name,
      role: entry.role || "ancestor",
      command: entry.command,
      alive: entry.alive,
    })),
);

const browserRows = computed<TreeRow[]>(() => {
  const children = new Map<number, ProcessView[]>();
  for (const process of props.processes) {
    const list = children.get(process.ppid) || [];
    list.push(process);
    children.set(process.ppid, list);
  }
  const root = props.processes.find((process) => process.pid === props.rootPid);
  if (!root) return [];
  const result: TreeRow[] = [];
  const queue: { process: ProcessView; depth: number }[] = [{ process: root, depth: 0 }];
  const seen = new Set<number>();
  while (queue.length && result.length < 220) {
    const { process, depth } = queue.shift()!;
    if (seen.has(process.pid)) continue;
    seen.add(process.pid);
    result.push({
      key: `p-${process.pid}`,
      depth,
      pid: process.pid,
      name: process.name,
      role: process.role,
      command: process.command,
      alive: true,
    });
    for (const child of children.get(process.pid) || []) {
      queue.push({ process: child, depth: depth + 1 });
    }
  }
  return result;
});
</script>

<template>
  <div class="process-tree">
    <section>
      <div class="tree-heading">
        <h3>Source evidence</h3>
        <span class="badge" :class="`badge-confidence-${confidence}`">{{ confidence }}</span>
      </div>
      <p v-if="!sourceRows.length" class="tree-empty">No observable ancestor chain. Origin is not reconstructed after the fact.</p>
      <div v-for="row in sourceRows" :key="row.key" class="tree-row" :class="`tree-depth-${Math.min(row.depth, 6)}`">
        <span class="tree-branch" aria-hidden="true"></span>
        <span class="dot" :class="row.alive ? 'dot-live' : 'dot-stale'"></span>
        <span class="tree-role">{{ row.role }}</span>
        <span class="tree-name">{{ row.name }}</span>
        <span class="tree-pid mono">{{ row.pid }}</span>
        <code class="tree-command" :title="row.command">{{ row.command }}</code>
      </div>
      <div class="tree-row tree-root" :class="`tree-depth-${Math.min(sourceRows.length, 6)}`">
        <span class="tree-branch" aria-hidden="true"></span>
        <span class="dot dot-live"></span>
        <span class="tree-role">browser root</span>
        <span class="tree-name">Chromium</span>
        <span class="tree-pid mono">{{ rootPid }}</span>
      </div>
    </section>

    <section>
      <div class="tree-heading">
        <h3>Verified process tree</h3>
        <span class="tree-count">{{ browserRows.length }} processes</span>
      </div>
      <div v-for="row in browserRows" :key="row.key" class="tree-row" :class="`tree-depth-${Math.min(row.depth, 6)}`">
        <span class="tree-branch" aria-hidden="true"></span>
        <span class="dot dot-live"></span>
        <span class="tree-role">{{ row.role }}</span>
        <span class="tree-name">{{ row.name }}</span>
        <span class="tree-pid mono">{{ row.pid }}</span>
        <code class="tree-command" :title="row.command">{{ row.command }}</code>
      </div>
    </section>
  </div>
</template>
