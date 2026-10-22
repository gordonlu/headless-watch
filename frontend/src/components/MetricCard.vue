<script setup lang="ts">
defineProps<{
  label: string;
  value: string;
  unit?: string;
  sub: string;
  tag: string;
  footLeft: string;
  footRight: string;
  accent?: "active" | "cpu" | "machine" | "risk";
  alert?: boolean;
}>();
</script>

<template>
  <article class="kpi" :class="[`kpi-${accent || 'active'}`, { 'is-alert': alert }]">
    <div class="kpi-top">
      <svg class="kpi-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <template v-if="accent === 'cpu'"><rect x="6" y="6" width="12" height="12" rx="2"/><rect x="9" y="9" width="6" height="6" rx="1"/><path d="M9 3v3m6-3v3M9 18v3m6-3v3M3 9h3m-3 6h3m12-6h3m-3 6h3"/></template>
        <template v-else-if="accent === 'machine'"><path d="M12 3v9h9A9 9 0 1 1 12 3Z"/><path d="M15 3.5A9 9 0 0 1 20.5 9H15Z"/></template>
        <template v-else-if="accent === 'risk'"><path d="M7 3h7l5 5v13H5V3h2Zm7 0v6h5M8 13h8m-8 4h5"/></template>
        <template v-else><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18"/></template>
      </svg>
      <span class="kpi-label">{{ label }}</span>
      <span class="kpi-tag">{{ tag }}</span>
    </div>
    <div class="kpi-value">{{ value }}<small v-if="unit">{{ unit }}</small></div>
    <div class="kpi-sub">{{ sub }}</div>
    <div class="kpi-foot"><span>{{ footLeft }}</span><span>{{ footRight }}</span></div>
  </article>
</template>
