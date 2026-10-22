<script setup lang="ts">
import { computed } from "vue";
import type { SessionView } from "../types";
import { fmtBytes, fmtCores, fmtDuration, fmtPct } from "../format";
import { openInspector, requestConfirm } from "../store";
import StatusBadges from "./StatusBadges.vue";

const props = defineProps<{ session: SessionView }>();

const memoryValue = computed(() => {
  if (props.session.memory.pss_sum !== null) return fmtBytes(props.session.memory.pss_sum);
  return `~${fmtBytes(props.session.memory.rss_sum)}`;
});
const memoryNote = computed(() => {
  const memory = props.session.memory;
  if (memory.pss_sum !== null) {
    const coverage = memory.pss_coverage;
    const coverageText = coverage ? ` · ${coverage.sampled}/${coverage.total} sampled` : "";
    return `PSS ${memory.pss_quality}${coverageText} · ${memory.pss_age_s ?? "—"}s ago`;
  }
  return "ΣRSS upper bound · PSS not sampled yet";
});
const cpuValue = computed(() =>
  props.session.cpu.cores === null ? "Calculating" : `${fmtCores(props.session.cpu.cores)} cores`,
);
const cpuNote = computed(() => {
  if (props.session.cpu.cores === null) return "First per-PID delta pending";
  return `${fmtPct(props.session.cpu.single_core_pct)} one core · ${fmtPct(props.session.cpu.machine_pct)} machine`;
});
const spark = computed(() => {
  const points = props.session.history.filter((point) => Number.isFinite(point.cores));
  if (points.length < 2) return [];
  const max = Math.max(...points.map((point) => point.cores), 0.01);
  return points.map((point, index) => {
    const x = (index / (points.length - 1)) * 100;
    const y = 30 - (point.cores / max) * 26;
    return [x, y] as const;
  });
});
const sparkLine = computed(() => spark.value.map(([x, y]) => `${x.toFixed(2)},${y.toFixed(2)}`).join(" "));
const sparkArea = computed(() =>
  spark.value.length
    ? `0,30 ${sparkLine.value} 100,30`
    : "",
);
const cdpLabel = computed(() => {
  const cdp = props.session.cdp;
  if (cdp.mode === "pipe") return "CDP pipe only";
  if (cdp.mode === "tcp" && cdp.state === "available") return `CDP :${cdp.port} · ${cdp.tabs.length} tab${cdp.tabs.length === 1 ? "" : "s"}`;
  if (cdp.mode === "tcp") return "CDP checking";
  return "No debug endpoint";
});
const lifecycleNote = computed(() => {
  if (props.session.status.suspected_leftover) return "Launcher exited; browser remains";
  if (props.session.status.parent_exited) return "Parent exited while browser remains";
  if (props.session.preexisting) return "Observed before monitor start";
  return "Observed live";
});

function inspect() {
  openInspector(props.session.id);
}

function keydown(event: KeyboardEvent) {
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    inspect();
  }
}
</script>

<template>
  <article
    class="session-card"
    :class="[`risk-${session.status.risk}`, { 'is-terminating': session.status.terminating, 'is-stale': session.status.stale }]"
    tabindex="0"
    role="button"
    :aria-label="`${session.browser}, PID ${session.pid}, ${session.source.summary}. Open Inspector.`"
    @click="inspect"
    @keydown="keydown"
  >
    <div class="session-main">
      <header class="session-head">
        <div class="browser-mark" aria-hidden="true">CH</div>
        <div class="session-title">
          <div class="session-title-line">
            <h3>{{ session.browser }}</h3>
            <span class="session-pid">PID {{ session.pid }} · {{ session.source.summary }}</span>
          </div>
          <StatusBadges :session="session" />
        </div>
        <span class="risk-chip" :class="`risk-chip-${session.status.risk}`">{{ session.status.risk === "warning" ? "Attention" : session.status.risk === "info" ? "Observed" : "Normal" }}</span>
      </header>

      <div class="session-metrics">
        <div class="metric">
          <span class="metric-label">CPU</span>
          <span class="metric-value">{{ cpuValue }}</span>
          <span class="metric-sub">{{ cpuNote }}</span>
        </div>
        <div class="metric">
          <span class="metric-label">Memory</span>
          <span class="metric-value">{{ memoryValue }}</span>
          <span class="metric-sub">{{ memoryNote }}</span>
        </div>
        <div class="metric">
          <span class="metric-label">Runtime</span>
          <span class="metric-value">{{ fmtDuration(session.lifetime_s) }}</span>
          <span class="metric-sub">{{ session.process_count }} processes · {{ lifecycleNote }}</span>
        </div>
      </div>

      <footer class="session-foot">
        <span>Origin · <strong>{{ session.source.launcher }}</strong> · {{ session.source.confidence }}</span>
        <span>{{ cdpLabel }}</span>
        <span v-if="session.action_notice" class="action-notice">{{ session.action_notice }}</span>
        <span class="card-actions">
          <button class="ghost-btn small" type="button" @click.stop="inspect">Inspect</button>
          <button
            v-if="session.force_available"
            class="danger-btn small"
            type="button"
            @click.stop="requestConfirm(session, 'kill')"
          >Force kill</button>
          <button
            class="stop-btn small"
            type="button"
            :disabled="!session.can_terminate"
            @click.stop="requestConfirm(session, 'term')"
          >Stop</button>
        </span>
      </footer>
    </div>

    <div class="spark-wrap" aria-hidden="true">
      <span class="spark-label">CPU · last 2 min</span>
      <svg v-if="spark.length > 1" class="spark" viewBox="0 0 100 30" preserveAspectRatio="none">
        <line v-for="line in [6, 13, 20, 27]" :key="line" x1="0" x2="100" :y1="line" :y2="line" stroke="var(--chart-grid)" stroke-width="0.5" />
        <polygon :points="sparkArea" fill="var(--spark-fill)" />
        <polyline :points="sparkLine" fill="none" stroke="var(--spark)" stroke-width="1.4" vector-effect="non-scaling-stroke" />
      </svg>
      <div v-else class="spark-empty">Collecting samples</div>
      <div class="spark-foot">
        <span>{{ session.cpu.quality }}</span>
        <span>PSS {{ session.memory.pss_quality }}</span>
      </div>
    </div>
  </article>
</template>
