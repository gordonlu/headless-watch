<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { ApiError, terminateBatch } from "../api";
import type { SessionView } from "../types";
import { refreshStore, toast } from "../store";

const props = defineProps<{ sessions: SessionView[] }>();
const emit = defineEmits<{ close: [] }>();

const dialog = ref<HTMLDialogElement | null>(null);
const checked = ref(false);
const busy = ref(false);
const error = ref("");

const count = computed(() => props.sessions.length);

watch(
  () => props.sessions,
  async (value) => {
    await nextTick();
    if (!dialog.value) return;
    if (value.length && !dialog.value.open) {
      checked.value = false;
      error.value = "";
      dialog.value.showModal();
    } else if (!value.length && dialog.value.open) {
      dialog.value.close();
    }
  },
  { immediate: true },
);

function close() {
  if (!busy.value) emit("close");
}

function backdrop(event: MouseEvent) {
  if (event.target === dialog.value) close();
}

async function confirm() {
  if (!props.sessions.length || !checked.value) return;
  busy.value = true;
  error.value = "";
  try {
    const outcome = await terminateBatch(
      props.sessions.map((session) => ({ id: session.id, identity: session.identity })),
      "term",
    );
    const stopped = outcome.results.filter((item) => item.ok).length;
    toast(
      `Stopped ${stopped} of ${outcome.count} selected session${outcome.count === 1 ? "" : "s"}.`,
      outcome.ok ? "success" : "warning",
    );
    emit("close");
    await refreshStore();
  } catch (err) {
    error.value =
      err instanceof ApiError
        ? err.message
        : err instanceof Error
          ? err.message
          : "Batch termination failed.";
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <dialog ref="dialog" class="modal-dialog confirm-dialog" aria-labelledby="batch-title" @close="close" @click="backdrop">
    <div v-if="sessions.length" class="modal-shell">
      <header class="modal-head">
        <div>
          <p class="eyebrow">Verified batch action</p>
          <h2 id="batch-title">Stop {{ count }} browser session{{ count === 1 ? "" : "s" }}?</h2>
          <p>Each session is re-verified independently before any signal is sent. Rejected sessions do not affect the others.</p>
        </div>
        <button class="icon-btn" type="button" aria-label="Cancel batch action" :disabled="busy" @click="close">×</button>
      </header>
      <div class="modal-body">
        <ul class="batch-list">
          <li v-for="session in sessions" :key="session.id" class="batch-row">
            <strong>{{ session.browser }}</strong>
            <span>PID {{ session.pid }}</span>
            <span>{{ session.source.summary }}</span>
            <span>{{ session.process_count }} processes</span>
          </li>
        </ul>
        <p class="notice notice-warning">
          SIGTERM is sent to each verified browser subtree only. Launchers, IDEs, and unrelated Chrome processes are not included. This cannot be undone.
        </p>
        <label class="check-row" for="batch-check">
          <input id="batch-check" v-model="checked" type="checkbox" :disabled="busy" />
          <span>I understand the impact and want to stop the {{ count }} selected verified session{{ count === 1 ? "" : "s" }}.</span>
        </label>
        <p v-if="error" class="form-error" role="alert">{{ error }}</p>
      </div>
      <footer class="modal-actions">
        <button class="ghost-btn" type="button" :disabled="busy" @click="close">Cancel</button>
        <button class="danger-btn" type="button" :disabled="!checked || busy" @click="confirm">
          {{ busy ? "Verifying…" : `Stop ${count} · SIGTERM` }}
        </button>
      </footer>
    </div>
  </dialog>
</template>
