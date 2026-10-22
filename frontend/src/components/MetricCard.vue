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
      <span class="kpi-label">{{ label }}</span>
      <span class="kpi-tag">{{ tag }}</span>
    </div>
    <div class="kpi-value">{{ value }}<small v-if="unit">{{ unit }}</small></div>
    <div class="kpi-sub">{{ sub }}</div>
    <div class="kpi-foot"><span>{{ footLeft }}</span><span>{{ footRight }}</span></div>
  </article>
</template>
