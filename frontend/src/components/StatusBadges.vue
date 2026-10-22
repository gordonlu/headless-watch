<script setup lang="ts">
import type { SessionView } from "../types";

defineProps<{ session: SessionView }>();
</script>

<template>
  <span class="badges">
    <span class="badge" :class="session.status.lifecycle === 'ended' ? 'badge-muted' : 'badge-live'">
      {{ session.status.lifecycle === "ended" ? "Exited" : "Active" }}
    </span>
    <span class="badge" :class="`badge-confidence-${session.source.confidence}`">
      {{ session.source.confidence }}
    </span>
    <span v-if="session.preexisting" class="badge badge-muted">Pre-existing</span>
    <span v-if="session.status.stale" class="badge badge-warning">Stale data</span>
    <span v-if="session.status.terminating" class="badge badge-warning">Stopping</span>
    <span v-if="session.status.high_cpu" class="badge badge-danger">High CPU</span>
    <span v-if="session.status.suspected_leftover" class="badge badge-warning">Suspected leftover</span>
    <span v-else-if="session.status.launcher_exited" class="badge badge-info">Launcher exited</span>
    <span v-else-if="session.status.parent_exited" class="badge badge-info">Parent exited</span>
    <span v-if="session.status.muted_until" class="badge badge-muted">Muted</span>
  </span>
</template>
