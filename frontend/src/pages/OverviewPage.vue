<script setup lang="ts">
import { computed } from "vue";
import type { PageName } from "../types";
import { fmtAgo, fmtBytes, fmtCores, fmtDuration, fmtPct } from "../format";
import { store } from "../store";
import MetricCard from "../components/MetricCard.vue";
import HistoryChart from "../components/HistoryChart.vue";
import SessionCard from "../components/SessionCard.vue";
import EventTimeline from "../components/EventTimeline.vue";

const emit = defineEmits<{ navigate: [page: PageName] }>();

const overview = computed(() => store.overview);
const summary = computed(() => overview.value?.summary);
const sessions = computed(() => overview.value?.sessions ?? []);
const events = computed(() => overview.value?.events ?? []);
const chartPoints = computed(() => overview.value?.system_history ?? []);
const recentEnded = computed(() => overview.value?.recent_ended ?? []);

function endedSource(source: unknown): string {
  const value = source as Record<string, unknown> | undefined;
  if (typeof value?.summary === "string" && value.summary) return value.summary;
  const launcher = String(value?.launcher || "Unknown");
  const driver = String(value?.driver || "Unknown");
  if (launcher === "Unknown" && driver === "Unknown") return "Unknown origin";
  if (launcher === "Unknown") return driver;
  if (driver === "Unknown") return launcher;
  return `${launcher} / ${driver}`;
}

function openEnded(id: string) {
  store.pendingHistoryId = id;
  emit("navigate", "history");
}
const cpuNote = "Per-PID tick deltas, never changing tree totals.";
const machineNote = "Denominator is the live online CPU count.";
const memoryNote = "ΣRSS may double-count shared pages; ΣPSS is sampled slowly.";
const chartWindow = computed(() => {
  const points = chartPoints.value;
  if (points.length < 2) return "No samples";
  return `Last ${fmtDuration(points[points.length - 1].t - points[0].t)} · ${points.length} points`;
});
const cpuSub = computed(() => {
  if (!summary.value) return "Waiting for samples";
  if (summary.value.calculating_sessions) {
    return `${summary.value.calculating_sessions} session${summary.value.calculating_sessions === 1 ? "" : "s"} calculating`;
  }
  return `${fmtPct(summary.value.cpu_single_core_pct)} of one core`;
});
const memoryFoot = computed(() => {
  if (!summary.value) return "ΣRSS upper bound";
  if (summary.value.pss_sum !== null) {
    return `PSS ${fmtBytes(summary.value.pss_sum)} · ${summary.value.pss_age_s ?? "—"}s ago`;
  }
  return `ΣRSS ~${fmtBytes(summary.value.rss_sum)} · upper bound`;
});
</script>

<template>
  <div class="page page-overview">
    <section class="kpis" aria-label="Key metrics">
      <MetricCard
        label="Active sessions"
        :value="String(summary?.active_sessions ?? 0)"
        sub="Headless browser roots under observation"
        tag="Trees"
        :foot-left="`${summary?.process_count ?? 0} total processes`"
        foot-right="grouped by root identity"
        accent="active"
      />
      <MetricCard
        label="CPU cores"
        :value="fmtCores(summary?.cpu_cores)"
        unit=" cores"
        :sub="cpuSub"
        tag="Live delta"
        :foot-left="`${fmtPct(summary?.cpu_machine_pct)} of machine`"
        :foot-right="`${summary?.online_cpus ?? '—'} online CPUs`"
        accent="cpu"
      />
      <MetricCard
        label="System CPU share"
        :value="summary ? fmtPct(summary.cpu_machine_pct) : '—'"
        sub="Denominator is the current online CPU count"
        tag="Machine"
        :foot-left="`${summary?.online_cpus ?? '—'} CPUs`"
        foot-right="never a static 16"
        accent="machine"
      />
      <MetricCard
        label="Suspected leftovers"
        :value="String(summary?.suspected_leftovers ?? 0)"
        :sub="summary?.parent_exited ? `${summary.parent_exited} parent exit observed` : 'No launcher exit observed'"
        tag="Risk"
        :foot-left="`${summary?.high_cpu ?? 0} high CPU`"
        :foot-right="memoryFoot"
        accent="risk"
        :alert="Boolean(summary?.suspected_leftovers || summary?.high_cpu)"
      />
    </section>

    <section class="chart-grid" aria-label="Browser telemetry">
      <article class="panel chart-panel compact" aria-labelledby="chart-cores-title">
        <div class="panel-head">
          <div>
            <p class="eyebrow">Telemetry · cores</p>
            <h3 id="chart-cores-title">CPU cores</h3>
          </div>
          <span class="chart-window">{{ chartWindow }}</span>
        </div>
        <div class="chart-meta">
          <div class="chart-legend">
            <span class="legend-item"><span class="legend-swatch swatch-mint"></span>CPU cores</span>
          </div>
          <span class="chart-window">{{ summary?.online_cpus ?? "—" }} online CPUs</span>
        </div>
        <HistoryChart :points="chartPoints" metric="cores" />
        <div class="chart-foot"><span>{{ cpuNote }}</span><span>1 core = 100%</span></div>
      </article>

      <article class="panel chart-panel compact" aria-labelledby="chart-machine-title">
        <div class="panel-head">
          <div>
            <p class="eyebrow">Telemetry · share</p>
            <h3 id="chart-machine-title">% of machine</h3>
          </div>
          <span class="chart-window">{{ chartWindow }}</span>
        </div>
        <div class="chart-meta">
          <div class="chart-legend">
            <span class="legend-item"><span class="legend-swatch swatch-cyan"></span>% of machine</span>
          </div>
          <span class="chart-window">{{ summary?.online_cpus ?? "—" }} CPUs</span>
        </div>
        <HistoryChart :points="chartPoints" metric="machine" />
        <div class="chart-foot"><span>{{ machineNote }}</span><span>{{ fmtPct(summary?.cpu_machine_pct) }} now</span></div>
      </article>

      <article class="panel chart-panel compact" aria-labelledby="chart-memory-title">
        <div class="panel-head">
          <div>
            <p class="eyebrow">Telemetry · memory</p>
            <h3 id="chart-memory-title">Memory</h3>
          </div>
          <span class="chart-window">{{ chartWindow }}</span>
        </div>
        <div class="chart-meta">
          <div class="chart-legend">
            <span class="legend-item"><span class="legend-swatch swatch-cyan"></span>ΣRSS (MiB)</span>
            <span class="legend-item"><span class="legend-swatch swatch-amber"></span>ΣPSS (MiB)</span>
          </div>
          <span class="chart-window">PSS {{ overview?.settings.pss_interval_s ?? "—" }}s</span>
        </div>
        <HistoryChart :points="chartPoints" metric="memory" />
        <div class="chart-foot"><span>{{ memoryNote }}</span><span>{{ fmtBytes(summary?.pss_sum ?? summary?.rss_sum) }} {{ summary?.pss_sum !== null ? "PSS" : "RSS" }}</span></div>
      </article>
    </section>

    <section class="section" aria-labelledby="active-title">
      <div class="section-head">
        <div>
          <p class="eyebrow">Live inventory</p>
          <h2 id="active-title">Active sessions <span class="count">{{ sessions.length }}</span></h2>
        </div>
        <button class="ghost-btn" type="button" @click="emit('navigate', 'live')">Open live view</button>
      </div>
      <div v-if="sessions.length" class="session-grid">
        <SessionCard v-for="session in sessions.slice(0, 6)" :key="session.id" :session="session" />
      </div>
      <div v-else class="empty-state">
        <strong>No headless browsers detected · Monitoring is active</strong>
        <p>Start a headless Chrome/Chromium process and it will appear within one scan interval. Existing browsers are shown as pre-existing with unknown origin.</p>
      </div>
    </section>

    <section v-if="recentEnded.length" class="section" aria-labelledby="ended-title">
      <div class="section-head">
        <div>
          <p class="eyebrow">Persistent history</p>
          <h2 id="ended-title">Recently exited <span class="count">{{ recentEnded.length }}</span></h2>
          <p class="section-sub">Stored in SQLite. Click a session to open its full source evidence and action audit.</p>
        </div>
        <button class="ghost-btn" type="button" @click="emit('navigate', 'history')">View all history</button>
      </div>
      <div class="recent-list">
        <button v-for="item in recentEnded" :key="item.id" class="recent-row" type="button" @click="openEnded(item.id)">
          <span class="recent-main"><strong>{{ item.browser }}</strong><small>PID {{ item.pid }}</small></span>
          <span class="recent-meta">{{ endedSource(item.source) }} · ended {{ fmtAgo(item.ended_at, store.nowMs) }} · ran {{ fmtDuration((item.ended_at || item.launched_at) - item.launched_at) }}</span>
          <span class="recent-facts">
            <span>{{ fmtCores(item.peak_cpu) }} peak cores</span>
            <span>{{ fmtBytes(item.latest_pss ?? item.latest_rss) }} {{ item.latest_pss !== null ? "PSS" : "RSS" }}</span>
            <span>{{ item.latest_process_count }} processes</span>
          </span>
        </button>
      </div>
    </section>

    <section class="panel activity-panel" aria-labelledby="activity-title">
      <div class="panel-head">
        <div>
          <p class="eyebrow">Audit trail</p>
          <h2 id="activity-title">Recent activity</h2>
          <p class="panel-sub">Launch, lifecycle, risk and action events stored in SQLite.</p>
        </div>
        <button class="ghost-btn small" type="button" @click="emit('navigate', 'history')">View history</button>
      </div>
      <EventTimeline :events="events" :limit="8" />
    </section>
  </div>
</template>
