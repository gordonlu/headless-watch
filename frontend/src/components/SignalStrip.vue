<script setup lang="ts">
import { computed } from "vue";
import type { SystemPoint } from "../types";
import { fmtCores } from "../format";

const props = defineProps<{ points: SystemPoint[] }>();

const geometry = computed(() => {
  const points = props.points.slice(-80).filter((point) => Number.isFinite(point.cores));
  if (points.length < 2) return null;
  const max = Math.max(...points.map((point) => point.cores), 0.01) * 1.15;
  const coords = points.map((point, index) => {
    const x = (index / (points.length - 1)) * 1000;
    const y = 68 - (point.cores / max) * 56;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  });
  return {
    line: coords.join(" "),
    area: `0,76 ${coords.join(" ")} 1000,76`,
    current: points[points.length - 1].cores,
  };
});
</script>

<template>
  <div class="signal-strip">
    <div class="signal-strip-head">
      <span>CPU trace</span>
      <strong>{{ geometry ? fmtCores(geometry.current) : "—" }} cores</strong>
    </div>
    <svg v-if="geometry" viewBox="0 0 1000 80" preserveAspectRatio="none" role="img" aria-label="CPU core trend for active sessions">
      <line v-for="line in [12, 28, 44, 60]" :key="line" x1="0" x2="1000" :y1="line" :y2="line" stroke="var(--chart-grid)" stroke-width="1" />
      <polygon :points="geometry.area" fill="var(--signal-fill)" />
      <polyline :points="geometry.line" fill="none" stroke="var(--signal)" stroke-width="1.5" vector-effect="non-scaling-stroke" />
    </svg>
    <div v-else class="signal-strip-empty">Waiting for a second sample…</div>
  </div>
</template>
