<script setup lang="ts">
import { computed, ref } from "vue";
import type { SessionView } from "../types";
import { fmtBytes, fmtCores, fmtDuration, fmtPct } from "../format";
import { openInspector, requestConfirm, sessions, stopLabel, store } from "../store";
import HistoryChart from "../components/HistoryChart.vue";
import RadarScope from "../components/RadarScope.vue";
import EventTimeline from "../components/EventTimeline.vue";
import SignalStrip from "../components/SignalStrip.vue";

const metric = ref<"cores" | "machine" | "memory">("cores");
const overview = computed(() => store.overview);
const summary = computed(() => overview.value?.summary);
const points = computed(() => overview.value?.system_history ?? []);
const events = computed(() => overview.value?.events ?? []);
const tape = computed(() =>
  [...sessions.value].sort((left, right) => (right.cpu.cores ?? 0) - (left.cpu.cores ?? 0)),
);
const openCodeCount = computed(() => sessions.value.filter((session) => session.source.launcher === "OpenCode").length);
const codexCount = computed(() => sessions.value.filter((session) => session.source.launcher === "Codex").length);

function riskClass(session: SessionView): string {
  if (session.status.risk === "warning") return "alert";
  if (session.status.risk === "info") return "info";
  return "live";
}
</script>

<template>
  <div class="terminal-overview">
    <section class="scope-readouts" aria-label="System readouts">
      <article class="scope-readout">
        <span class="scope-label">Active sessions</span>
        <strong class="scope-value">{{ summary?.active_sessions ?? 0 }}</strong>
        <span class="scope-sub">{{ summary?.process_count ?? 0 }} processes</span>
      </article>
      <article class="scope-readout">
        <span class="scope-label">CPU cores</span>
        <strong class="scope-value">{{ fmtCores(summary?.cpu_cores) }}</strong>
        <span class="scope-sub">{{ fmtPct(summary?.cpu_single_core_pct) }} one core · {{ fmtPct(summary?.cpu_machine_pct) }} machine</span>
      </article>
      <article class="scope-readout">
        <span class="scope-label">Machine share</span>
        <strong class="scope-value">{{ fmtPct(summary?.cpu_machine_pct) }}</strong>
        <span class="scope-sub">{{ summary?.online_cpus ?? "—" }} online CPUs</span>
      </article>
      <article class="scope-readout">
        <span class="scope-label">Suspected leftovers</span>
        <strong class="scope-value">{{ summary?.suspected_leftovers ?? 0 }}</strong>
        <span class="scope-sub">{{ summary?.parent_exited ?? 0 }} parent exits · {{ summary?.high_cpu ?? 0 }} high CPU</span>
      </article>
    </section>

    <div class="scope-main">
      <section class="scope-panel radar-panel" aria-labelledby="radar-title">
        <header class="scope-panel-head">
          <h2 id="radar-title">Signal field</h2>
          <span>distance / CPU (1 core+ = centre) · size / processes · angle / session</span>
        </header>
        <RadarScope :sessions="sessions" />
        <footer class="scope-panel-foot">
          <span class="scope-legend-item"><i class="scope-dot opencode"></i>OpenCode</span>
          <span class="scope-legend-item"><i class="scope-dot codex"></i>Codex</span>
          <span class="scope-legend-item"><i class="scope-dot unknown"></i>Unknown</span>
          <span class="scope-foot-note">1 core+ = centre</span>
        </footer>
      </section>

      <div class="scope-side">
      <section class="scope-panel tape-panel" aria-labelledby="tape-title">
        <header class="scope-panel-head">
          <h2 id="tape-title">Signal log</h2>
          <a class="scope-link" href="#live">All sessions →</a>
        </header>
        <div class="scope-tape">
          <div
            v-for="session in tape"
            :key="session.id"
            class="tape-row"
            role="button"
            tabindex="0"
            :aria-label="`Open ${session.browser} PID ${session.pid}`"
            @click="openInspector(session.id)"
            @keydown.enter.prevent="openInspector(session.id)"
          >
            <span class="tape-status" :class="riskClass(session)"></span>
            <span class="tape-pid">P{{ session.pid }}</span>
            <span class="tape-source">{{ session.browser }} · {{ session.source.summary }}</span>
            <span class="tape-cpu">{{ fmtCores(session.cpu.cores) }} cores</span>
            <span class="tape-mem">{{ fmtBytes(session.memory.pss_sum ?? session.memory.rss_sum) }}</span>
            <span class="tape-run">{{ fmtDuration(session.lifetime_s) }}</span>
            <button
              class="tape-stop"
              type="button"
              :disabled="!session.can_terminate"
              @click.stop="requestConfirm(session, 'term')"
            >{{ stopLabel }}</button>
          </div>
          <div v-if="!tape.length" class="tape-empty">No headless browser signals detected.</div>
        </div>
        <SignalStrip :points="points" />
        <footer class="tape-footer">
          <span>SCAN {{ store.overview?.settings.interval_s ?? "—" }}s</span>
          <span>OPENCODE {{ openCodeCount }}</span>
          <span>CODEX {{ codexCount }}</span>
          <span>SUSPECTED {{ summary?.suspected_leftovers ?? 0 }}</span>
        </footer>
      </section>

      <section class="scope-panel scope-mini-events" aria-labelledby="trace-title">
        <header class="scope-panel-head">
          <h2 id="trace-title">Event trace</h2>
          <span>Latest</span>
        </header>
        <EventTimeline :events="events" :limit="4" compact />
      </section>
      </div>
    </div>

    <section class="scope-panel scope-wave" aria-labelledby="wave-title">
      <header class="scope-panel-head">
        <h2 id="wave-title">Signal history</h2>
        <div class="scope-tabs" role="group" aria-label="History metric">
          <button type="button" :class="{ 'is-active': metric === 'cores' }" @click="metric = 'cores'">Cores</button>
          <button type="button" :class="{ 'is-active': metric === 'machine' }" @click="metric = 'machine'">Machine</button>
          <button type="button" :class="{ 'is-active': metric === 'memory' }" @click="metric = 'memory'">Memory</button>
        </div>
      </header>
      <HistoryChart :points="points" :metric="metric" />
    </section>
  </div>
</template>
