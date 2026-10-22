<script setup lang="ts">
import type { EventRow } from "../types";
import { fmtClock, titleCase } from "../format";
import { openInspector } from "../store";

defineProps<{
  events: EventRow[];
  limit?: number;
  compact?: boolean;
}>();

function activate(event: EventRow) {
  if (event.session_id) openInspector(event.session_id);
}
</script>

<template>
  <ol class="timeline" :class="{ 'timeline-compact': compact }">
    <li v-for="event in events.slice(0, limit || 12)" :key="event.id" class="timeline-item">
      <span class="timeline-time">{{ fmtClock(event.t) }}</span>
      <span class="timeline-mark" :class="`severity-${event.severity}`" aria-hidden="true"></span>
      <div class="timeline-copy">
        <button
          class="timeline-message"
          type="button"
          :disabled="!event.session_id"
          @click="activate(event)"
        >{{ event.message }}</button>
        <div class="timeline-meta">
          <span>{{ titleCase(event.type) }}</span>
          <span v-if="event.session_id" class="mono">{{ event.session_id }}</span>
        </div>
      </div>
    </li>
    <li v-if="!events.length" class="timeline-empty">No events recorded in this run.</li>
  </ol>
</template>
