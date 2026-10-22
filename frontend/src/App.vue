<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import type { PageName } from "./types";
import { fmtAgo, fmtDuration } from "./format";
import { initStore, pendingAlerts, refreshStore, store } from "./store";
import OverviewPage from "./pages/OverviewPage.vue";
import LivePage from "./pages/LivePage.vue";
import HistoryPage from "./pages/HistoryPage.vue";
import AlertsPage from "./pages/AlertsPage.vue";
import SettingsPage from "./pages/SettingsPage.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import HelpDialog from "./components/HelpDialog.vue";
import ToastStack from "./components/ToastStack.vue";
import InspectorDrawer from "./components/InspectorDrawer.vue";

const pages: PageName[] = ["overview", "live", "history", "alerts", "settings"];
const page = ref<PageName>(readPage().page);
const refreshing = ref(false);
type Theme = "default" | "terminal";
const theme = ref<Theme>(readTheme());

const titles: Record<PageName, string> = {
  overview: "Invisible Browser Observatory",
  live: "Live Sessions",
  history: "History",
  alerts: "Alerts",
  settings: "Settings",
};

const pageTitle = computed(() => titles[page.value]);
const health = computed(() => store.overview?.health);
const summary = computed(() => store.overview?.summary);
const scannerState = computed(() => {
  if (!health.value) return "Starting";
  if (health.value.status === "degraded") return "Degraded";
  return store.connected ? "Scanner live" : "Reconnecting";
});
const scanAge = computed(() => {
  const generated = store.overview?.generated_at;
  return generated ? fmtAgo(generated, store.nowMs) : "Awaiting data";
});
const healthNote = computed(() => {
  if (store.error) return store.error;
  if (!health.value) return "Waiting for the first process scan.";
  const age = health.value.last_scan_age_s;
  if (age === null) return "Waiting for the first process scan.";
  return `Last process scan ${age < 2 ? "just now" : fmtDuration(age) + " ago"} · ${health.value.stale_sessions} stale.`;
});
const runtime = computed(() => {
  const settings = store.overview?.settings;
  if (!settings) return "Waiting for monitor settings…";
  return `Scan ${settings.interval_s}s · alert ${settings.cpu_alert_cores} cores / ${settings.cpu_alert_duration_s}s · PSS ${settings.pss_interval_s}s · v${store.overview?.version} · monitor PID ${health.value?.monitor_pid}`;
});

function readTheme(): Theme {
  const query = new URLSearchParams(window.location.search).get("theme");
  if (query === "terminal" || query === "default") return query;
  return localStorage.getItem("hw-theme") === "terminal" ? "terminal" : "default";
}

function applyTheme(value: Theme, persist = true) {
  document.documentElement.dataset.theme = value === "terminal" ? "terminal" : "";
  if (persist) localStorage.setItem("hw-theme", value);
}

function setTheme(value: Theme) {
  theme.value = value;
  applyTheme(value);
}

function readPage(): { page: PageName; session: string | null } {
  const raw = window.location.hash.replace("#", "");
  const [pagePart, query] = raw.split("?");
  const value = pagePart as PageName;
  const params = new URLSearchParams(query || "");
  return {
    page: pages.includes(value) ? value : "overview",
    session: params.get("session"),
  };
}

function applyHash() {
  const parsed = readPage();
  page.value = parsed.page;
  if (parsed.session) store.pendingInspectorId = parsed.session;
}

function go(next: PageName) {
  page.value = next;
  store.pendingInspectorId = null;
  if (window.location.hash !== `#${next}`) window.location.hash = next;
}

watch(
  () => store.overview?.sessions.map((session) => session.id).join(","),
  () => {
    const pending = store.pendingInspectorId;
    if (!pending) return;
    if (store.overview?.sessions.some((session) => session.id === pending)) {
      store.inspectorId = pending;
      store.pendingInspectorId = null;
    }
  },
);

async function refresh() {
  refreshing.value = true;
  await refreshStore();
  window.setTimeout(() => (refreshing.value = false), 350);
}

function onKeydown(event: KeyboardEvent) {
  const target = event.target as HTMLElement | null;
  const typing =
    target &&
    (target.tagName === "INPUT" ||
      target.tagName === "TEXTAREA" ||
      target.tagName === "SELECT" ||
      target.isContentEditable);
  if (event.key === "Escape") {
    store.inspectorId = null;
    store.confirm = null;
    store.helpOpen = false;
    return;
  }
  if (typing) return;
  if (event.key === "/") {
    event.preventDefault();
    go("live");
    void nextTick(() => {
      document.querySelector<HTMLInputElement>("[data-session-search]")?.focus();
    });
  } else if (event.key === "?") {
    event.preventDefault();
    store.helpOpen = true;
  }
}

onMounted(() => {
  applyTheme(theme.value, false);
  applyHash();
  void initStore();
  window.addEventListener("hashchange", applyHash);
  window.addEventListener("keydown", onKeydown);
});
</script>

<template>
  <a class="skip-link" href="#main">Skip to content</a>
  <div class="shell">
    <aside class="rail">
      <a class="brand" href="#overview" @click.prevent="go('overview')" aria-label="Headless Watch overview">
        <span class="brand-mark" aria-hidden="true"><span></span></span>
        <span class="brand-copy">
          <span class="brand-title">Headless Watch</span>
          <span class="brand-sub">Invisible browser control</span>
        </span>
      </a>

      <nav class="nav" aria-label="Dashboard sections">
        <p class="nav-label">Observe</p>
        <button class="nav-item" :class="{ 'is-active': page === 'overview' }" type="button" @click="go('overview')">
          <span class="nav-index">01</span>Overview
        </button>
        <button class="nav-item" :class="{ 'is-active': page === 'live' }" type="button" @click="go('live')">
          <span class="nav-index">02</span>Live sessions
          <span v-if="summary?.active_sessions" class="nav-count">{{ summary.active_sessions }}</span>
        </button>
        <button class="nav-item" :class="{ 'is-active': page === 'history' }" type="button" @click="go('history')">
          <span class="nav-index">03</span>History
        </button>
        <button class="nav-item" :class="{ 'is-active': page === 'alerts' }" type="button" @click="go('alerts')">
          <span class="nav-index">04</span>Alerts
          <span v-if="pendingAlerts.length" class="nav-count nav-count-alert">{{ pendingAlerts.length }}</span>
        </button>
        <p class="nav-label nav-label-spaced">Control</p>
        <button class="nav-item" :class="{ 'is-active': page === 'settings' }" type="button" @click="go('settings')">
          <span class="nav-index">05</span>Settings
        </button>
      </nav>

      <div class="rail-status" aria-live="polite">
        <div class="rail-label">Scanner</div>
        <div class="status-line">
          <span class="dot" :class="!store.connected ? 'dot-stale' : health?.status === 'degraded' ? 'dot-warning' : 'dot-live'"></span>
          <span>{{ scannerState }}</span>
        </div>
        <p class="rail-health">{{ healthNote }}</p>
        <dl class="rail-meta">
          <div><dt>Uptime</dt><dd>{{ fmtDuration(store.overview?.uptime_s) }}</dd></div>
          <div><dt>Interval</dt><dd>{{ store.overview?.settings.interval_s ?? "—" }}s</dd></div>
        </dl>
      </div>

      <p class="rail-note">
        <strong>Read-only by default.</strong> Stop and kill requests are re-verified against PID start time, owner and headless evidence before signaling.
      </p>
      <p class="rail-key"><kbd>/</kbd> search <kbd>?</kbd> shortcuts</p>
    </aside>

    <main class="main" id="main">
      <header class="topbar">
        <div>
          <p class="eyebrow">Local observability · 127.0.0.1</p>
          <h1>{{ pageTitle }}</h1>
        </div>
        <div class="topbar-right">
          <span class="scan-age">{{ scanAge }}</span>
          <span class="conn" :class="{ 'conn-offline': !store.connected }">
            <span class="conn-dot"></span>{{ store.connected ? "Live" : "Reconnecting" }}
          </span>
          <div class="seg-control theme-switch" role="group" aria-label="Interface theme">
            <button class="seg-btn" :class="{ 'is-active': theme === 'default' }" type="button" @click="setTheme('default')">Observatory</button>
            <button class="seg-btn" :class="{ 'is-active': theme === 'terminal' }" type="button" @click="setTheme('terminal')">Terminal</button>
          </div>
          <button class="ghost-btn" type="button" :disabled="refreshing" @click="refresh">
            {{ refreshing ? "Refreshing…" : "Refresh" }}
          </button>
        </div>
      </header>

      <div v-if="store.error" class="offline-banner" role="alert">
        <span class="offline-mark" aria-hidden="true"></span>
        <span><strong>Monitor unreachable.</strong> {{ store.error }} Showing the last known snapshot while reconnecting.</span>
      </div>

      <div v-if="store.loading" class="loading-panel" role="status">Loading the local process snapshot…</div>

      <template v-else>
        <OverviewPage v-if="page === 'overview'" @navigate="go" />
        <LivePage v-else-if="page === 'live'" />
        <HistoryPage v-else-if="page === 'history'" />
        <AlertsPage v-else-if="page === 'alerts'" />
        <SettingsPage v-else />
      </template>

      <footer class="footer">
        <span>{{ runtime }}</span>
        <span><strong>Local only.</strong> No cloud, no arbitrary CDP, no silent termination.</span>
      </footer>
    </main>
  </div>

  <InspectorDrawer />
  <ConfirmDialog />
  <HelpDialog />
  <ToastStack />
</template>
