<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import {
  downloadExport,
  getActions,
  getEvents,
  getHistory,
} from "../api";
import type { ActionRow, EventRow, HistorySession, SessionView, StatusView } from "../types";
import { fmtBytes, fmtCores, fmtDateTime, fmtDuration, titleCase } from "../format";
import { refreshStore, store, toast } from "../store";

const rows = ref<HistorySession[]>([]);
const loading = ref(true);
const error = ref("");
const search = ref("");
const sourceFilter = ref("");
const selected = ref<HistorySession | null>(null);
const detailEvents = ref<EventRow[]>([]);
const detailActions = ref<ActionRow[]>([]);
const detailLoading = ref(false);
const dialog = ref<HTMLDialogElement | null>(null);

async function loadHistory() {
  loading.value = true;
  error.value = "";
  try {
    const response = await getHistory({ limit: 200 });
    rows.value = response.sessions;
    const pending = store.pendingHistoryId;
    if (pending) {
      store.pendingHistoryId = null;
      const row = rows.value.find((item) => item.id === pending);
      if (row) void openDetail(row);
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Cannot load session history.";
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  search.value = sessionStorage.getItem("hw-history-search") || "";
  sourceFilter.value = sessionStorage.getItem("hw-history-source") || "";
  void loadHistory();
});
watch([search, sourceFilter], () => {
  sessionStorage.setItem("hw-history-search", search.value);
  sessionStorage.setItem("hw-history-source", sourceFilter.value);
});
watch(
  () => store.overview?.events[0]?.id,
  () => {
    const latest = store.overview?.events[0];
    if (latest?.type === "session_ended") void loadHistory();
  },
);

const sources = computed(() => {
  const values = new Set<string>();
  for (const row of rows.value) {
    const view = row.final_view;
    values.add(view?.source.launcher || String((row.source as Record<string, unknown>).launcher || "Unknown"));
  }
  return Array.from(values).sort();
});

const filtered = computed(() => {
  const query = search.value.trim().toLowerCase();
  return rows.value.filter((row) => {
    const view = row.final_view;
    const launcher = view?.source.launcher || String((row.source as Record<string, unknown>).launcher || "Unknown");
    if (sourceFilter.value && launcher !== sourceFilter.value) return false;
    if (!query) return true;
    return `${row.browser} ${row.root_pid} ${row.id} ${launcher} ${row.command}`
      .toLowerCase()
      .includes(query);
  });
});

const span = computed(() => {
  const list = filtered.value;
  if (!list.length) return { start: 0, end: 1 };
  const start = Math.min(...list.map((row) => row.launched_at || row.first_seen_at));
  const end = Math.max(
    ...list.map((row) => row.ended_at || row.last_seen_at),
    store.nowMs / 1000,
  );
  return { start, end: end > start ? end : start + 1 };
});

function ganttX(timestamp: number): number {
  return ((timestamp - span.value.start) / (span.value.end - span.value.start)) * 1000;
}

function ganttWidth(row: HistorySession): number {
  const start = ganttX(row.launched_at || row.first_seen_at);
  const end = ganttX(row.ended_at || row.last_seen_at);
  return Math.max(8, end - start);
}

function statusOf(row: HistorySession): StatusView | Record<string, unknown> {
  return row.final_view?.status || row.status || {};
}

function numeric(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function launcherExit(row: HistorySession): number | null {
  const view = row.final_view;
  if (view) {
    return view.status.launcher_exited_at || view.parent.exited_at || view.status.watch_exited_at || null;
  }
  const status = row.status as Record<string, unknown>;
  return numeric(status.launcher_exited_at) || numeric(status.watch_exited_at);
}

function sourceSummary(row: HistorySession): string {
  if (row.final_view) return row.final_view.source.summary;
  const source = row.source as Record<string, unknown>;
  const launcher = String(source.launcher || "Unknown");
  const driver = String(source.driver || "Unknown");
  if (launcher === "Unknown" && driver === "Unknown") return "Unknown origin";
  if (launcher === "Unknown") return driver;
  if (driver === "Unknown") return launcher;
  return `${launcher} / ${driver}`;
}

async function openDetail(row: HistorySession) {
  selected.value = row;
  detailEvents.value = [];
  detailActions.value = [];
  await nextTick();
  if (dialog.value && !dialog.value.open) dialog.value.showModal();
  detailLoading.value = true;
  try {
    const [events, actions] = await Promise.all([getEvents(row.id, 120), getActions(500)]);
    detailEvents.value = events.events;
    detailActions.value = actions.actions.filter((action) => action.session_id === row.id);
  } catch (err) {
    toast(err instanceof Error ? err.message : "Cannot load session detail.", "error");
  } finally {
    detailLoading.value = false;
  }
}

function closeDetail() {
  selected.value = null;
  if (dialog.value?.open) dialog.value.close();
}

function backdrop(event: MouseEvent) {
  if (event.target === dialog.value) closeDetail();
}

async function exportFile(format: "json" | "csv") {
  try {
    await downloadExport(format, "sessions");
    toast(`Exported sessions as ${format.toUpperCase()}.`, "success");
  } catch (err) {
    toast(err instanceof Error ? err.message : "Export failed.", "error");
  }
}

const detailView = computed<SessionView | null>(() => selected.value?.final_view ?? null);
</script>

<template>
  <div class="page">
    <section class="section section-first">
      <div class="section-head">
        <div>
          <p class="eyebrow">Persistent history</p>
          <h2>Session lifecycles <span class="count">{{ filtered.length }}</span></h2>
          <p class="section-sub">SQLite WAL records source snapshots, agent exit timing, peak CPU and action audits.</p>
        </div>
        <div class="session-tools">
          <div class="search-wrap">
            <label class="sr-only" for="history-search">Search history</label>
            <input id="history-search" v-model="search" type="search" placeholder="Search browser, PID, origin…" />
          </div>
          <select v-model="sourceFilter" class="select" aria-label="Filter history source">
            <option value="">All sources</option>
            <option v-for="item in sources" :key="item" :value="item">{{ item }}</option>
          </select>
          <button class="ghost-btn" type="button" @click="loadHistory">Refresh</button>
          <button class="ghost-btn small" type="button" @click="exportFile('json')">Export JSON</button>
          <button class="ghost-btn small" type="button" @click="exportFile('csv')">CSV</button>
        </div>
      </div>

      <p v-if="error" class="form-error" role="alert">{{ error }}</p>
      <div v-if="loading" class="loading-panel">Loading persisted sessions…</div>
      <div v-else-if="filtered.length" class="history-list">
        <button v-for="row in filtered" :key="row.id" class="history-row" type="button" @click="openDetail(row)">
          <span class="history-info">
            <span class="history-title">{{ row.browser }} <small>PID {{ row.root_pid }}</small></span>
            <span class="history-meta">{{ sourceSummary(row) }} · {{ fmtDateTime(row.launched_at) }} · {{ fmtDuration((row.ended_at || row.last_seen_at) - row.launched_at) }}</span>
          </span>
          <span class="history-gantt" aria-hidden="true">
            <svg viewBox="0 0 1000 16" preserveAspectRatio="none">
              <rect x="0" y="6" width="1000" height="4" rx="2" fill="rgba(38,53,74,.7)" />
              <rect
                :x="ganttX(row.launched_at || row.first_seen_at)"
                y="4"
                :width="ganttWidth(row)"
                height="8"
                rx="3"
                :fill="statusOf(row).risk === 'warning' ? 'var(--chart-3)' : 'var(--chart-1)'"
              />
              <line
                v-if="launcherExit(row)"
                :x1="ganttX(launcherExit(row)!)"
                :x2="ganttX(launcherExit(row)!)"
                y1="1"
                y2="15"
                stroke="var(--chart-2)"
                stroke-width="1.5"
                stroke-dasharray="2 2"
              />
            </svg>
          </span>
          <span class="history-facts">
            <span><strong>{{ fmtCores(row.peak_cpu) }}</strong> peak cores</span>
            <span><strong>{{ fmtBytes(row.latest_pss ?? row.latest_rss) }}</strong> {{ row.latest_pss ? "PSS" : "RSS" }}</span>
            <span>{{ row.latest_process_count }} processes</span>
          </span>
        </button>
      </div>
      <div v-else class="empty-state">
        <strong>No ended sessions recorded yet.</strong>
        <p>When a monitored browser exits, its source evidence and metric history remain available here.</p>
      </div>
      <div class="legend-row">
        <span><i class="legend-line legend-mint"></i> browser lifetime</span>
        <span><i class="legend-line legend-cyan"></i> observed launcher exit</span>
      </div>
    </section>
  </div>

  <dialog ref="dialog" class="modal-dialog history-dialog" aria-labelledby="history-detail-title" @close="selected = null" @click="backdrop">
    <div v-if="selected" class="modal-shell">
      <header class="modal-head">
        <div>
          <p class="eyebrow">Historical evidence</p>
          <h2 id="history-detail-title">{{ selected.browser }} · PID {{ selected.root_pid }}</h2>
          <p>{{ sourceSummary(selected) }} · {{ fmtDateTime(selected.launched_at) }} → {{ fmtDateTime(selected.ended_at || selected.last_seen_at) }}</p>
        </div>
        <button class="icon-btn" type="button" aria-label="Close history detail" @click="closeDetail">×</button>
      </header>
      <div class="modal-body history-detail">
        <dl class="fact-grid">
          <div><dt>Session</dt><dd class="mono">{{ selected.id }}</dd></div>
          <div><dt>Discovery</dt><dd>{{ selected.discovery_mode }}</dd></div>
          <div><dt>Runtime</dt><dd>{{ fmtDuration((selected.ended_at || selected.last_seen_at) - selected.launched_at) }}</dd></div>
          <div><dt>Peak CPU</dt><dd>{{ fmtCores(selected.peak_cpu) }} cores</dd></div>
          <div><dt>Memory</dt><dd>{{ fmtBytes(selected.latest_pss ?? selected.latest_rss) }} {{ selected.latest_pss ? "PSS" : "RSS upper bound" }}</dd></div>
          <div><dt>Processes</dt><dd>{{ selected.latest_process_count }}</dd></div>
          <div><dt>Command</dt><dd class="mono wrap">{{ selected.command }}</dd></div>
          <div><dt>cgroup</dt><dd class="mono wrap">{{ selected.cgroup || "Not available" }}</dd></div>
        </dl>

        <section v-if="detailView" class="history-section">
          <h3>Source chain</h3>
          <div v-for="entry in detailView.ancestry" :key="`${entry.pid}-${entry.start_ticks}`" class="ancestor-row">
            <span class="dot" :class="entry.alive ? 'dot-live' : 'dot-stale'"></span>
            <strong>{{ entry.name }}</strong>
            <span class="mono">{{ entry.pid }}</span>
            <span>{{ entry.role || "ancestor" }}</span>
            <code>{{ entry.command }}</code>
          </div>
          <p v-if="!detailView.ancestry.length" class="tree-empty">No ancestor chain was observable at discovery.</p>
        </section>

        <section class="history-section">
          <h3>Events</h3>
          <p v-if="detailLoading">Loading events…</p>
          <ul v-else class="compact-list">
            <li v-for="event in detailEvents" :key="event.id">
              <span class="timeline-time">{{ fmtDateTime(event.t) }}</span>
              <span class="badge" :class="`badge-${event.severity}`">{{ titleCase(event.type) }}</span>
              <span>{{ event.message }}</span>
            </li>
            <li v-if="!detailEvents.length" class="compact-empty">No events retained for this session.</li>
          </ul>
        </section>

        <section class="history-section">
          <h3>Action audit</h3>
          <ul v-if="detailActions.length" class="compact-list">
            <li v-for="action in detailActions" :key="action.id">
              <span class="timeline-time">{{ fmtDateTime(action.t) }}</span>
              <span class="badge" :class="action.result === 'stopped' || action.result === 'killed' ? 'badge-live' : 'badge-warning'">{{ titleCase(action.result) }}</span>
              <span>{{ action.message }}</span>
            </li>
          </ul>
          <p v-else class="tree-empty">No stop or kill action was recorded.</p>
        </section>
      </div>
      <footer class="modal-actions">
        <span class="fine-print">This history is stored locally with redacted commands and URLs.</span>
        <button class="ghost-btn" type="button" @click="closeDetail">Close</button>
      </footer>
    </div>
  </dialog>
</template>
