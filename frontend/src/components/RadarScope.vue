<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { SessionView } from "../types";

const props = defineProps<{ sessions: SessionView[] }>();

const canvas = ref<HTMLCanvasElement | null>(null);
let observer: ResizeObserver | null = null;
let frame = 0;
let started = 0;

const sourceColor = (session: SessionView): string => {
  const launcher = session.source.launcher;
  if (launcher === "OpenCode") return "#59f7ff";
  if (launcher === "Codex") return "#ffb547";
  if (launcher === "Claude Code") return "#ff8fb3";
  if (launcher === "Unknown") return "#7f8b91";
  return "#7da7ff";
};

const angleFor = (value: string): number => {
  let hash = 0;
  for (const char of value) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return ((hash % 360) * Math.PI) / 180;
};

function draw(timestamp: number) {
  const element = canvas.value;
  if (!element) return;
  const rect = element.getBoundingClientRect();
  if (rect.width < 20 || rect.height < 20) return;
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const width = Math.floor(rect.width);
  const height = Math.floor(rect.height);
  if (element.width !== Math.floor(width * dpr) || element.height !== Math.floor(height * dpr)) {
    element.width = Math.floor(width * dpr);
    element.height = Math.floor(height * dpr);
  }
  const ctx = element.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);
  const cx = width / 2;
  const cy = height / 2;
  const radius = Math.min(width, height) / 2 - 28;
  if (radius < 20) return;

  if (started === 0) started = timestamp;
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const sweep = reduced ? Math.PI * 0.18 : ((timestamp - started) / 1800) % (Math.PI * 2);

  const halo = ctx.createRadialGradient(cx, cy, radius * 0.2, cx, cy, radius);
  halo.addColorStop(0, "rgba(13, 26, 29, 0.1)");
  halo.addColorStop(1, "rgba(0, 0, 0, 0.25)");
  ctx.fillStyle = halo;
  ctx.beginPath();
  ctx.arc(cx, cy, radius, 0, Math.PI * 2);
  ctx.fill();

  ctx.strokeStyle = "#2a444a";
  ctx.lineWidth = 1;
  for (let ring = 1; ring <= 4; ring += 1) {
    ctx.beginPath();
    ctx.arc(cx, cy, (radius * ring) / 4, 0, Math.PI * 2);
    ctx.stroke();
  }
  ctx.strokeStyle = "rgba(72, 110, 116, 0.7)";
  for (let spoke = 0; spoke < 12; spoke += 1) {
    const angle = (spoke / 12) * Math.PI * 2;
    ctx.beginPath();
    ctx.moveTo(cx + Math.cos(angle) * 8, cy + Math.sin(angle) * 8);
    ctx.lineTo(cx + Math.cos(angle) * radius, cy + Math.sin(angle) * radius);
    ctx.stroke();
  }

  if (!reduced) {
    const trail = 1.05;
    // PPI afterglow fades by angle behind the beam, not by distance from the
    // centre. A conic gradient gives the real radar trail instead of a glowing
    // triangular fill that is brightest at the origin.
    if (typeof ctx.createConicGradient === "function") {
      const cone = ctx.createConicGradient(sweep - trail, cx, cy);
      cone.addColorStop(0, "rgba(89, 247, 255, 0)");
      cone.addColorStop(0.05, "rgba(89, 247, 255, 0.04)");
      cone.addColorStop(0.12, "rgba(89, 247, 255, 0.16)");
      cone.addColorStop(0.158, "rgba(89, 247, 255, 0.34)");
      cone.addColorStop(0.172, "rgba(89, 247, 255, 0.14)");
      cone.addColorStop(0.2, "rgba(89, 247, 255, 0)");
      cone.addColorStop(1, "rgba(89, 247, 255, 0)");
      ctx.fillStyle = cone;
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.arc(cx, cy, radius, sweep - trail, sweep);
      ctx.closePath();
      ctx.fill();
    }
    ctx.strokeStyle = "rgba(137, 255, 255, 0.98)";
    ctx.lineWidth = 1.6;
    ctx.shadowColor = "#59f7ff";
    ctx.shadowBlur = 12;
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(cx + Math.cos(sweep) * radius, cy + Math.sin(sweep) * radius);
    ctx.stroke();
    ctx.shadowBlur = 0;
    ctx.lineWidth = 1;
  }

  ctx.strokeStyle = "rgba(89, 247, 255, 0.28)";
  ctx.beginPath();
  ctx.moveTo(cx - 5, cy);
  ctx.lineTo(cx + 5, cy);
  ctx.moveTo(cx, cy - 5);
  ctx.lineTo(cx, cy + 5);
  ctx.stroke();

  const maxProcesses = Math.max(...props.sessions.map((session) => session.process_count), 1);
  const usedAngles: number[] = [];
  for (const session of props.sessions) {
    let angle = angleFor(session.id);
    let guard = 0;
    while (
      usedAngles.some((used) => Math.abs(Math.atan2(Math.sin(used - angle), Math.cos(used - angle))) < 0.3) &&
      guard < 24
    ) {
      angle += 0.28;
      guard += 1;
    }
    usedAngles.push(angle);
    // Absolute scale: one fully busy core sits at the centre; idle sessions stay at the edge.
    const load = Math.min(1, Math.max(0, session.cpu.cores ?? 0));
    const scale = 0.18 + 0.72 * (1 - load);
    const x = cx + Math.cos(angle) * radius * scale;
    const y = cy + Math.sin(angle) * radius * scale;
    const color = sourceColor(session);
    const size = 3.5 + Math.min(5, (session.process_count / maxProcesses) * 5);
    const angularDistance = Math.abs(Math.atan2(Math.sin(sweep - angle), Math.cos(sweep - angle)));
    const afterglow = Math.max(0, 1 - angularDistance / 1.2);
    ctx.shadowColor = color;
    ctx.shadowBlur = 8 + afterglow * 18;
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.arc(x, y, size, 0, Math.PI * 2);
    ctx.fill();
    ctx.shadowBlur = 0;
    ctx.strokeStyle = color;
    ctx.globalAlpha = 0.4;
    ctx.beginPath();
    ctx.arc(x, y, size + 4, 0, Math.PI * 2);
    ctx.stroke();
    ctx.globalAlpha = 1;
    ctx.fillStyle = "#edf8fa";
    ctx.font = '12px "IBM Plex Mono", ui-monospace, monospace';
    ctx.textAlign = "left";
    ctx.fillText(`P${session.pid}`, x + 10, y + 4);
  }

  ctx.fillStyle = "#a9bcc0";
  ctx.font = '12px "IBM Plex Mono", ui-monospace, monospace';
  ctx.textAlign = "center";
  ctx.fillText(
    `${props.sessions.length} ${props.sessions.length === 1 ? "SIGNAL" : "SIGNALS"}`,
    cx,
    cy + radius + 18,
  );
  if (!props.sessions.length) {
    ctx.fillStyle = "#59f7ff";
    ctx.font = '12px "IBM Plex Mono", ui-monospace, monospace';
    ctx.fillText("NO SIGNAL", cx, cy - 12);
  }
}

function animate(timestamp: number) {
  draw(timestamp);
  if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    frame = window.requestAnimationFrame(animate);
  }
}

onMounted(() => {
  observer = new ResizeObserver(() => draw(performance.now()));
  if (canvas.value) observer.observe(canvas.value);
  started = 0;
  animate(performance.now());
});

onBeforeUnmount(() => {
  observer?.disconnect();
  if (frame) window.cancelAnimationFrame(frame);
});

watch(() => props.sessions, () => draw(performance.now()), { deep: true });
</script>

<template>
  <div class="radar-canvas">
    <canvas ref="canvas" role="img" aria-label="Radar view of active headless browser sessions, with radius by process count and angle by session identity"></canvas>
  </div>
</template>
