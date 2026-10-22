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

function cssVar(name: string, fallback: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

function alpha(color: string, value: number): string {
  if (color.startsWith("#") && (color.length === 7 || color.length === 4)) {
    const hex = color.length === 4
      ? color.slice(1).split("").map((part) => part + part).join("")
      : color.slice(1);
    const red = Number.parseInt(hex.slice(0, 2), 16);
    const green = Number.parseInt(hex.slice(2, 4), 16);
    const blue = Number.parseInt(hex.slice(4, 6), 16);
    return `rgba(${red}, ${green}, ${blue}, ${value})`;
  }
  return color;
}

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
  const chart1 = cssVar("--chart-1", "#4be4ba");
  const chart2 = cssVar("--chart-2", "#70c9ff");
  const chart3 = cssVar("--chart-3", "#ffc06d");
  const axis = cssVar("--chart-axis", "#7187a1");
  const grid = cssVar("--chart-grid", "rgba(38,53,74,.7)");
  const baseline = cssVar("--chart-baseline", "rgba(58,84,106,.9)");
  const chartFont = cssVar("--chart-font", "9px ui-monospace, monospace");
  let series: { values: (number | null)[]; color: string; fill: boolean; dash: number[] }[];
  let maxValue = 0;
  if (metric === "memory") {
    const rss = points.map((point) => (finite(point.rss_sum) ? point.rss_sum / 1048576 : null));
    const pss = points.map((point) =>
      finite(point.pss_sum) && point.pss_sum >= 0 ? point.pss_sum / 1048576 : null,
    );
    series = [
      { values: rss, color: chart2, fill: true, dash: [] },
      { values: pss, color: chart3, fill: false, dash: [4, 3] },
    ];
    maxValue = Math.max(...rss.filter(finite), ...pss.filter(finite), 0);
  } else if (metric === "machine") {
    series = [
      {
        values: points.map((point) => (finite(point.machine_pct) ? point.machine_pct : null)),
        color: chart2,
        fill: true,
        dash: [],
      },
    ];
    maxValue = Math.max(...points.map((point) => point.machine_pct).filter(finite), 0);
  } else {
    series = [
      {
        values: points.map((point) => (finite(point.cores) ? point.cores : null)),
        color: chart1,
        fill: true,
        dash: [],
      },
    ];
    maxValue = Math.max(...points.map((point) => point.cores).filter(finite), 0);
  }
  const max = niceMax(maxValue * 1.15);
  const firstT = points[0].t;
  const lastT = points[points.length - 1].t;
  const span = lastT - firstT;
  const axisTime = (timestamp: number) =>
    span < 60 ? `+${(timestamp - firstT).toFixed(1)}s` : fmtClock(timestamp);
  const xFor = (index: number) =>
    left + (lastT === firstT ? plotW / 2 : ((points[index].t - firstT) / (lastT - firstT)) * plotW);
  const yFor = (value: number) => top + plotH - (Math.max(0, value) / max) * plotH;

  context.lineWidth = 1;
  context.textBaseline = "middle";
  for (let index = 0; index <= 4; index += 1) {
    const value = (max * index) / 4;
    const y = top + plotH - (index / 4) * plotH;
    context.strokeStyle = index === 0 ? baseline : grid;
    context.beginPath();
    context.moveTo(left, y + 0.5);
    context.lineTo(left + plotW, y + 0.5);
    context.stroke();
    context.fillStyle = axis;
    context.font = chartFont;
    context.textAlign = "right";
    context.fillText(axisLabel(value, metric), left - 8, y);
  }

  context.textBaseline = "top";
  const positions = new Set(
    span < 10 && width < 460
      ? [0, points.length - 1]
      : width < 460
        ? [0, Math.floor((points.length - 1) / 2), points.length - 1]
        : [
            0,
            Math.floor((points.length - 1) / 3),
            Math.floor((2 * (points.length - 1)) / 3),
            points.length - 1,
          ],
  );
  const labelPositions = [...positions]
    .map((index) => ({
      index,
      x: index === 0 ? left : index === points.length - 1 ? left + plotW : xFor(index),
    }))
    .sort((a, b) => a.x - b.x);
  const keptLabels: typeof labelPositions = [];
  for (const item of labelPositions) {
    const isFirst = item.index === 0;
    const isLast = item.index === points.length - 1;
    if (!isFirst && !isLast) {
      const firstX = keptLabels[0]?.x ?? left;
      const lastX = left + plotW;
      if (item.x - firstX < 48 || lastX - item.x < 48) continue;
    }
    if (keptLabels.some((kept) => Math.abs(kept.x - item.x) < 48)) continue;
    keptLabels.push(item);
  }
  for (const item of keptLabels) {
    const start = item.index === 0;
    const end = item.index === points.length - 1;
    context.textAlign = start ? "left" : end ? "right" : "center";
    context.fillStyle = axis;
    context.font = chartFont;
    context.fillText(axisTime(points[item.index].t), item.x, top + plotH + 9);
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
      gradient.addColorStop(0, alpha(item.color, metric === "memory" ? 0.12 : 0.16));
      gradient.addColorStop(1, alpha(item.color, 0));
      context.fillStyle = gradient;
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
