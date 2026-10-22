<script setup lang="ts">
import { computed } from "vue";
import type { SessionView } from "../types";
import { fmtBytes, fmtCores, fmtDuration, fmtPct } from "../format";
import { isWindows, openInspector, requestConfirm, stopLabel } from "../store";
import StatusBadges from "./StatusBadges.vue";

const props = defineProps<{ sessions: SessionView[]; selected: string[] }>();
const emit = defineEmits<{ toggle: [id: string]; toggleAll: [ids: string[]] }>();

const allSelected = computed(() => props.sessions.length > 0 && props.sessions.every((session) => props.selected.includes(session.id)));


function keydown(event: KeyboardEvent, session: SessionView) {
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    openInspector(session.id);
  }
}
</script>

<template>
  <div class="table-wrap">
    <table class="session-table">
      <thead>
        <tr>
          <th class="check-cell">
            <input
              type="checkbox"
              :checked="allSelected"
              aria-label="Select all visible sessions"
              @click.stop
              @change="emit('toggleAll', sessions.map((session) => session.id))"
            />
          </th>
          <th>Browser / origin</th>
          <th>PID</th>
          <th>CPU</th>
          <th>Machine</th>
          <th>Memory</th>
          <th>Processes</th>
          <th>Runtime</th>
          <th>Risk</th>
          <th><span class="sr-only">Actions</span></th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="session in sessions"
          :key="session.id"
          tabindex="0"
          :class="[`risk-${session.status.risk}`, { 'is-selected': selected.includes(session.id) }]"
          @click="openInspector(session.id)"
          @keydown="keydown($event, session)"
        >
          <td class="check-cell">
            <input
              type="checkbox"
              :checked="selected.includes(session.id)"
              :aria-label="`Select ${session.browser} PID ${session.pid}`"
              @click.stop="emit('toggle', session.id)"
            />
          </td>
          <td>
            <div class="table-browser">
              <span class="browser-mark mini" aria-hidden="true">CH</span>
              <div>
                <strong>{{ session.browser }}</strong>
                <span>{{ session.source.summary }}</span>
              </div>
            </div>
          </td>
          <td class="mono">{{ session.pid }}</td>
          <td>
            <strong class="mono">{{ session.cpu.cores === null ? "Calculating" : fmtCores(session.cpu.cores) }}</strong>
            <small>{{ fmtPct(session.cpu.single_core_pct) }} one core</small>
          </td>
          <td class="mono">{{ fmtPct(session.cpu.machine_pct) }}</td>
          <td>
            <strong class="mono">{{ fmtBytes(session.memory.pss_sum ?? session.memory.rss_sum) }}</strong>
            <small>{{ isWindows ? "Working Set upper bound" : session.memory.pss_sum !== null ? "PSS" : "RSS upper bound" }}</small>
          </td>
          <td class="mono">{{ session.process_count }}</td>
          <td class="mono">{{ fmtDuration(session.lifetime_s) }}</td>
          <td><StatusBadges :session="session" /></td>
          <td class="table-actions">
            <button class="ghost-btn small" type="button" @click.stop="openInspector(session.id)">Inspect</button>
            <button
              class="stop-btn small"
              type="button"
              :disabled="!session.can_terminate"
              @click.stop="requestConfirm(session, 'term')"
            >{{ stopLabel }}</button>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
