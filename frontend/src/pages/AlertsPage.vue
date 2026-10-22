<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { acknowledgeAlert, getAlerts, muteAlert } from "../api";
import type { EventRow } from "../types";
import { fmtAgo, fmtDateTime, titleCase } from "../format";
import { openInspector, store, toast } from "../store";

type Filter = "pending" | "acknowledged" | "muted" | "all";
const alerts = ref<EventRow[]>([]);
const loading = ref(true);
const error = ref("");
const filter = ref<Filter>("pending");
const busy = ref<number | null>(null);

async function loadAlerts() {
  try {
    const response = await getAlerts(200);
    alerts.value = response.alerts;
    error.value = "";
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Cannot load alerts.";
  } finally {
    loading.value = false;
  }
}

onMounted(loadAlerts);
watch(
  () => store.overview?.events[0]?.id,
  () => {
    const latest = store.overview?.events[0];
    if (latest && ["high_cpu", "suspected_leftover", "action_failed"].includes(latest.type)) {
      void loadAlerts();
    }
  },
);

function isMuted(event: EventRow): boolean {
  return Boolean(event.muted_until && event.muted_until > Date.now() / 1000);
}

const filtered = computed(() => {
  return alerts.value.filter((event) => {
    if (filter.value === "pending") return !event.acknowledged && !isMuted(event);
    if (filter.value === "acknowledged") return event.acknowledged;
    if (filter.value === "muted") return isMuted(event);
    return true;
  });
});

const counts = computed(() => ({
  pending: alerts.value.filter((event) => !event.acknowledged && !isMuted(event)).length,
  acknowledged: alerts.value.filter((event) => event.acknowledged).length,
  muted: alerts.value.filter((event) => isMuted(event)).length,
  all: alerts.value.length,
}));

async function acknowledge(event: EventRow) {
  busy.value = event.id;
  try {
    await acknowledgeAlert(event.id);
    event.acknowledged = true;
    toast("Alert acknowledged.", "success");
  } catch (err) {
    toast(err instanceof Error ? err.message : "Cannot acknowledge alert.", "error");
  } finally {
    busy.value = null;
  }
}

async function mute(event: EventRow) {
  busy.value = event.id;
  try {
    await muteAlert(event.id, 30);
    event.muted_until = Date.now() / 1000 + 30 * 60;
    toast("Alert muted for 30 minutes.", "success");
  } catch (err) {
    toast(err instanceof Error ? err.message : "Cannot mute alert.", "error");
  } finally {
    busy.value = null;
  }
}
</script>

<template>
  <div class="page">
    <section class="section section-first">
      <div class="section-head">
        <div>
          <p class="eyebrow">Reliability center</p>
          <h2>Alerts <span class="count">{{ counts.pending }}</span></h2>
          <p class="section-sub">Sustained CPU and source-exit signals, with persisted evidence and acknowledgement.</p>
        </div>
        <button class="ghost-btn" type="button" @click="loadAlerts">Refresh</button>
      </div>

      <div class="filter-row" role="group" aria-label="Filter alerts">
        <button class="filter-btn" :class="{ 'is-active': filter === 'pending' }" type="button" @click="filter = 'pending'">Pending <span class="filter-count">{{ counts.pending }}</span></button>
        <button class="filter-btn" :class="{ 'is-active': filter === 'acknowledged' }" type="button" @click="filter = 'acknowledged'">Acknowledged <span class="filter-count">{{ counts.acknowledged }}</span></button>
        <button class="filter-btn" :class="{ 'is-active': filter === 'muted' }" type="button" @click="filter = 'muted'">Muted <span class="filter-count">{{ counts.muted }}</span></button>
        <button class="filter-btn" :class="{ 'is-active': filter === 'all' }" type="button" @click="filter = 'all'">All <span class="filter-count">{{ counts.all }}</span></button>
      </div>

      <p v-if="error" class="form-error" role="alert">{{ error }}</p>
      <div v-if="loading" class="loading-panel">Loading alert history…</div>
      <div v-else-if="filtered.length" class="alert-list">
        <article v-for="alert in filtered" :key="alert.id" class="alert-row" :class="{ 'is-ack': alert.acknowledged, 'is-muted': isMuted(alert) }">
          <span class="alert-mark" :class="`severity-${alert.severity}`" aria-hidden="true"></span>
          <div class="alert-copy">
            <div class="alert-title">
              <span class="badge" :class="`badge-${alert.severity}`">{{ titleCase(alert.type) }}</span>
              <span class="alert-time">{{ fmtDateTime(alert.t) }} · {{ fmtAgo(alert.t, store.nowMs) }}</span>
            </div>
            <p>{{ alert.message }}</p>
            <div class="alert-meta">
              <span v-if="alert.session_id" class="mono">{{ alert.session_id }}</span>
              <button
                v-if="alert.session_id && store.overview?.sessions.some((session) => session.id === alert.session_id)"
                class="link-btn"
                type="button"
                @click="openInspector(alert.session_id!)"
              >Inspect live session</button>
              <span v-if="isMuted(alert)">Muted until {{ fmtDateTime(alert.muted_until) }}</span>
              <span v-else-if="alert.acknowledged">Acknowledged</span>
            </div>
          </div>
          <div class="alert-actions">
            <button class="ghost-btn small" type="button" :disabled="alert.acknowledged || busy === alert.id" @click="acknowledge(alert)">Acknowledge</button>
            <button class="ghost-btn small" type="button" :disabled="isMuted(alert) || busy === alert.id" @click="mute(alert)">Mute 30m</button>
          </div>
        </article>
      </div>
      <div v-else class="empty-state">
        <strong>{{ filter === "pending" ? "No pending alerts." : "No alerts in this view." }}</strong>
        <p>High CPU alerts require a sustained threshold crossing. Source exits are observations, not automatic cleanup decisions.</p>
      </div>
    </section>
  </div>
</template>
