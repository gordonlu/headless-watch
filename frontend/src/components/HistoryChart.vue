<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { SystemPoint } from "../types";
import { fmtClock } from "../format";

const props = defineProps<{
  points: SystemPoint[];
  metric: "cores" | "machine" | "memory";
}>();

const canvas = ref<HTMLCanvasElement | null>(null);
let observer: ResizeObserver | null = null;

const finite = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value);

function niceMax(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return 1;
  const exponent = 10 ** Math.floor(Math.log10(value));
  const fraction = value / exponent;
  const step = fraction <= 1 ? 1 : fraction <= 2 ? 2 : fraction <= 5 ? 5 : 10;
  return step * exponent;
}

function axisLabel(value: number, metric: string): string {
  if (metric === "cores") return Number(value.toFixed(value < 1 ? 3 : 2)).toString();
  if (metric === "machine") return Number(value.toFixed(value < 1 ? 2 : 1)).toString();
  return `${Math.round(value)}`;
}

function draw() {
  const element = canvas.value;
  if (!element) return;
  const rect = element.getBoundingClientRect();
  if (rect.width < 10 || rect.height < 10) return;
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const width = Math.floor(rect.width);
  const height = Math.floor(rect.height);
  if (element.width !== Math.floor(width * dpr) || element.height !== Math.floor(height * dpr)) {
    element.width = Math.floor(width * dpr);
    element.height = Math.floor(height * dpr);
  }
  const context = element.getContext("2d");
  if (!context) return;
  context.setTransform(dpr, 0, 0, dpr, 0, 0);
  context.clearRect(0, 0, width, height);
  const points = props.points || [];
  if (points.length < 2) {
    context.fillStyle = "#7b8fa8";
    context.font = "11px ui-monospace, monospace";
    context.textAlign = "center";
    context.textBaseline = "middle";
    context.fillText("Waiting for a second sample…", width / 2, height / 2);
    return;
  }
  const left = 52;
  const right = 18;
  const top = 16;
  const bottom = 28;
  const plotW = Math.max(1, width - left - right);
  const plotH = Math.max(1, height - top - bottom);
  const metric = props.metric;
  let series: { values: (number | null)[]; color: string; fill: boolean; dash: number[] }[];
  let maxValue = 0;
  if (metric === "memory") {
    const rss = points.map((point) => (finite(point.rss_sum) ? point.rss_sum / 1048576 : null));
    const pss = points.map((point) =>
      finite(point.pss_sum) && point.pss_sum >= 0 ? point.pss_sum / 1048576 : null,
    );
    series = [
      { values: rss, color: "#70c9ff", fill: true, dash: [] },
      { values: pss, color: "#ffc06d", fill: false, dash: [4, 3] },
    ];
    maxValue = Math.max(...rss.filter(finite), ...pss.filter(finite), 0);
  } else if (metric === "machine") {
    series = [
      {
        values: points.map((point) => (finite(point.machine_pct) ? point.machine_pct : null)),
        color: "#70c9ff",
        fill: true,
        dash: [],
      },
    ];
    maxValue = Math.max(...points.map((point) => point.machine_pct).filter(finite), 0);
  } else {
    series = [
      {
        values: points.map((point) => (finite(point.cores) ? point.cores : null)),
        color: "#4be4ba",
        fill: true,
        dash: [],
      },
    ];
    maxValue = Math.max(...points.map((point) => point.cores).filter(finite), 0);
  }
  const max = niceMax(maxValue * 1.15);
  const firstT = points[0].t;
  const lastT = points[points.length - 1].t;
  const xFor = (index: number) =>
    left + (lastT === firstT ? plotW / 2 : ((points[index].t - firstT) / (lastT - firstT)) * plotW);
  const yFor = (value: number) => top + plotH - (Math.max(0, value) / max) * plotH;

  context.lineWidth = 1;
  context.textBaseline = "middle";
  for (let index = 0; index <= 4; index += 1) {
    const value = (max * index) / 4;
    const y = top + plotH - (index / 4) * plotH;
    context.strokeStyle = index === 0 ? "rgba(58,84,106,.9)" : "rgba(38,53,74,.7)";
    context.beginPath();
    context.moveTo(left, y + 0.5);
    context.lineTo(left + plotW, y + 0.5);
    context.stroke();
    context.fillStyle = "#7187a1";
    context.font = "9px ui-monospace, monospace";
    context.textAlign = "right";
    context.fillText(axisLabel(value, metric), left - 8, y);
  }

  context.textBaseline = "top";
  const positions = new Set(
    width < 460
      ? [0, Math.floor((points.length - 1) / 2), points.length - 1]
      : [
          0,
          Math.floor((points.length - 1) / 3),
          Math.floor((2 * (points.length - 1)) / 3),
          points.length - 1,
        ],
  );
  for (const index of positions) {
    const start = index === 0;
    const end = index === points.length - 1;
    context.textAlign = start ? "left" : end ? "right" : "center";
    context.fillStyle = "#7187a1";
    context.font = "9px ui-monospace, monospace";
    context.fillText(fmtClock(points[index].t), start ? left : end ? left + plotW : xFor(index), top + plotH + 9);
  }

  for (const item of series) {
    context.beginPath();
    let started = false;
    let firstX = 0;
    let lastX = 0;
    let lastY = 0;
    for (let index = 0; index < points.length; index += 1) {
      const value = item.values[index];
      if (!finite(value)) {
        started = false;
        continue;
      }
      const x = xFor(index);
      const y = yFor(value);
      if (!started) {
        context.moveTo(x, y);
        started = true;
        firstX = x;
      } else {
        context.lineTo(x, y);
      }
      lastX = x;
      lastY = y;
    }
    context.strokeStyle = item.color;
    context.lineWidth = 1.7;
    context.lineJoin = "round";
    context.lineCap = "round";
    context.setLineDash(item.dash);
    context.stroke();
    context.setLineDash([]);
    if (item.fill && lastX > firstX) {
      context.lineTo(lastX, top + plotH);
      context.lineTo(firstX, top + plotH);
      context.closePath();
      const gradient = context.createLinearGradient(0, top, 0, top + plotH);
      gradient.addColorStop(0, "rgba(75,228,186,.16)");
      gradient.addColorStop(1, "rgba(75,228,186,0)");
      context.fillStyle = metric === "memory" ? "rgba(112,201,255,.10)" : gradient;
      context.fill();
    }
  }
}

onMounted(() => {
  observer = new ResizeObserver(draw);
  if (canvas.value) observer.observe(canvas.value);
  draw();
});

onBeforeUnmount(() => observer?.disconnect());

watch(() => [props.points, props.metric], draw, { deep: true });
</script>

<template>
  <div class="chart-wrap">
    <canvas ref="canvas" class="chart-canvas" role="img" aria-label="Observed browser CPU and memory history"></canvas>
  </div>
</template>
