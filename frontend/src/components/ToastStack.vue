<script setup lang="ts">
import { dismissToast, store } from "../store";
</script>

<template>
  <div class="toast-stack" aria-live="polite" aria-atomic="false">
    <div v-for="toast in store.toasts" :key="toast.id" class="toast" :class="`toast-${toast.kind}`">
      <span class="toast-mark" aria-hidden="true"></span>
      <span>{{ toast.message }}</span>
      <button class="icon-btn toast-close" type="button" aria-label="Dismiss" @click="dismissToast(toast.id)">×</button>
    </div>
  </div>
</template>
