<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import type { SessionView } from "../types";
import { fmtCores } from "../format";
import { isWindows, sessions } from "../store";
import SessionCard from "../components/SessionCard.vue";
import SessionTable from "../components/SessionTable.vue";
import BatchConfirmDialog from "../components/BatchConfirmDialog.vue";

type ViewMode = "cards" | "table";
type RiskFilter = "all" | "risk" | "exited" | "unknown";

const search = ref("");
const sort = ref<"cpu" | "memory" | "runtime" | "recent" | "risk">("cpu");
const source = ref("");
const risk = ref<RiskFilter>("all");
const view = ref<ViewMode>("cards");
const selected = ref<string[]>([]);
const batchOpen = ref(false);

onMounted(() => {
  search.value = sessionStorage.getItem("hw-session-search") || "";
  sort.value = (sessionStorage.getItem("hw-session-sort") as typeof sort.value) || "cpu";
  risk.value = (sessionStorage.getItem("hw-session-risk") as RiskFilter) || "all";
  view.value = (sessionStorage.getItem("hw-session-view") as ViewMode) || "cards";
});

watch([search, sort, risk, view, source], () => {
  sessionStorage.setItem("hw-session-search", search.value);
  sessionStorage.setItem("hw-session-sort", sort.value);
  sessionStorage.setItem("hw-session-risk", risk.value);
  sessionStorage.setItem("hw-session-view", view.value);
});

const sources = computed(() => {
  const values = new Set<string>();
  for (const session of sessions.value) values.add(session.source.launcher);
  return Array.from(values).sort();
});

const filtered = computed(() => {
  const query = search.value.trim().toLowerCase();
  let rows = sessions.value.filter((session) => {
    if (source.value && session.source.launcher !== source.value) return false;
    if (risk.value === "risk" && session.status.risk !== "warning") return false;
    if (
      risk.value === "exited" &&
      !session.status.launcher_exited &&
      !session.status.parent_exited &&
      !session.status.driver_exited
    ) {
      return false;
    }
    if (risk.value === "unknown" && session.source.launcher !== "Unknown") return false;
    if (!query) return true;
    return `${session.browser} ${session.pid} ${session.id} ${session.source.summary} ${session.parent.name ?? ""}`
      .toLowerCase()
      .includes(query);
  });
  rows = [...rows];
  const riskOrder: Record<string, number> = { warning: 2, info: 1, none: 0 };
  rows.sort((left, right) => {
    switch (sort.value) {
      case "memory":
        return right.memory.rss_sum - left.memory.rss_sum;
      case "runtime":
        return right.lifetime_s - left.lifetime_s;
      case "recent":
        return right.first_seen_at - left.first_seen_at;
      case "risk":
        return (riskOrder[right.status.risk] ?? 0) - (riskOrder[left.status.risk] ?? 0);
      default:
        return (right.cpu.cores ?? -1) - (left.cpu.cores ?? -1);
    }
  });
  return rows;
});

const selectedSessions = computed(() =>
  sessions.value.filter((session) => selected.value.includes(session.id)),
);
const totalCores = computed(() =>
  filtered.value.reduce((total, session) => total + (session.cpu.cores ?? 0), 0),
);

function toggle(id: string) {
  const index = selected.value.indexOf(id);
  if (index >= 0) selected.value.splice(index, 1);
  else selected.value.push(id);
}

function toggleAll(ids: string[]) {
  const allSelected = ids.length > 0 && ids.every((id) => selected.value.includes(id));
  if (allSelected) {
    selected.value = selected.value.filter((id) => !ids.includes(id));
  } else {
    selected.value = Array.from(new Set([...selected.value, ...ids]));
  }
}

function closeBatch() {
  batchOpen.value = false;
  selected.value = [];
}

function clearFilters() {
  search.value = "";
  source.value = "";
  risk.value = "all";
  sort.value = "cpu";
}
</script>

<template>
  <div class="page">
    <section class="section section-first">
      <div class="section-head live-head">
        <div>
          <p class="eyebrow">Live inventory</p>
          <h2>Sessions <span class="count">{{ filtered.length }}</span></h2>
          <p class="section-sub">{{ fmtCores(totalCores) }} cores across the filtered browser trees.</p>
        </div>
        <div class="session-tools">
          <div class="search-wrap">
            <label class="sr-only" for="session-search">Search sessions</label>
            <input
              id="session-search"
              v-model="search"
              data-session-search
              type="search"
              placeholder="Search browser, PID, origin…"
              autocomplete="off"
              spellcheck="false"
            />
          </div>
          <label class="sr-only" for="session-source">Filter source</label>
          <select id="session-source" v-model="source" class="select">
            <option value="">All sources</option>
            <option v-for="item in sources" :key="item" :value="item">{{ item }}</option>
          </select>
          <label class="sr-only" for="session-sort">Sort sessions</label>
          <select id="session-sort" v-model="sort" class="select">
            <option value="cpu">Sort: CPU</option>
            <option value="memory">Sort: Memory</option>
            <option value="runtime">Sort: Runtime</option>
            <option value="recent">Sort: Recent</option>
            <option value="risk">Sort: Risk</option>
          </select>
          <div class="seg-control view-control" role="group" aria-label="Session view">
            <button class="seg-btn" :class="{ 'is-active': view === 'cards' }" type="button" @click="view = 'cards'">Cards</button>
            <button class="seg-btn" :class="{ 'is-active': view === 'table' }" type="button" @click="view = 'table'">Table</button>
          </div>
        </div>
      </div>

      <div class="filter-row" role="group" aria-label="Filter sessions">
        <button class="filter-btn" :class="{ 'is-active': risk === 'all' }" type="button" @click="risk = 'all'">All</button>
        <button class="filter-btn" :class="{ 'is-active': risk === 'risk' }" type="button" @click="risk = 'risk'">Needs attention</button>
        <button class="filter-btn" :class="{ 'is-active': risk === 'exited' }" type="button" @click="risk = 'exited'">Launcher exited</button>
        <button class="filter-btn" :class="{ 'is-active': risk === 'unknown' }" type="button" @click="risk = 'unknown'">Unknown origin</button>
        <button v-if="search || source || risk !== 'all'" class="filter-reset" type="button" @click="clearFilters">Clear filters</button>
      </div>

      <div v-if="selected.length" class="batch-bar">
        <span><strong>{{ selected.length }}</strong> selected</span>
        <span v-if="selected.length > 20" class="batch-warning">Batch limit is 20 sessions.</span>
        <span class="batch-spacer"></span>
        <button class="ghost-btn small" type="button" @click="selected = []">Clear</button>
        <button class="danger-btn small" type="button" :disabled="selected.length > 20" @click="batchOpen = true">
          {{ isWindows ? 'Force stop selected' : 'Stop selected · SIGTERM' }}
        </button>
      </div>

      <div v-if="filtered.length && view === 'cards'" class="session-grid">
        <SessionCard v-for="session in filtered" :key="session.id" :session="session" />
      </div>
      <SessionTable
        v-else-if="filtered.length"
        :sessions="filtered"
        :selected="selected"
        @toggle="toggle"
        @toggle-all="toggleAll"
      />
      <div v-else-if="!sessions.length" class="empty-state">
        <strong>No headless browsers detected · Monitoring is active</strong>
        <p>Start a headless Chrome/Chromium process and it will appear within one scan interval.</p>
      </div>
      <div v-else class="empty-state">
        <strong>No sessions match these filters.</strong>
        <p>Clear the search and filters to return to the complete live inventory.</p>
        <button class="ghost-btn" type="button" @click="clearFilters">Clear filters</button>
      </div>
    </section>
  </div>
  <BatchConfirmDialog v-if="batchOpen" :sessions="selectedSessions" @close="closeBatch" />
</template>
