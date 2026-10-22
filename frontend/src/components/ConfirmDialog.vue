<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { ApiError, terminateSession } from "../api";
import { closeConfirm, refreshStore, store, toast } from "../store";

const dialog = ref<HTMLDialogElement | null>(null);
const checked = ref(false);
const busy = ref(false);
const error = ref("");

const request = computed(() => store.confirm);
const modeLabel = computed(() => (request.value?.mode === "kill" ? "Force kill" : "Stop gracefully"));
const verb = computed(() => (request.value?.mode === "kill" ? "SIGKILL" : "SIGTERM"));

watch(
  request,
  async (value) => {
    await nextTick();
    if (!dialog.value) return;
    if (value && !dialog.value.open) {
      checked.value = false;
      error.value = "";
      dialog.value.showModal();
    } else if (!value && dialog.value.open) {
      dialog.value.close();
    }
  },
  { immediate: true },
);

function close() {
  if (!busy.value) store.confirm = null;
}

function backdrop(event: MouseEvent) {
  if (event.target === dialog.value) close();
}

async function confirmAction() {
  if (!request.value || !checked.value) return;
  busy.value = true;
  error.value = "";
  try {
    const outcome = await terminateSession(
      request.value.session.id,
      {
        pid: request.value.session.identity.pid,
        start_ticks: request.value.session.identity.start_ticks,
      },
      request.value.mode,
    );
    toast(outcome.message, outcome.ok ? "success" : "warning");
    store.confirm = null;
    await refreshStore();
  } catch (err) {
    const message = err instanceof ApiError ? err.message : err instanceof Error ? err.message : "Termination failed.";
    if (message.includes("no longer active") || message.includes("already terminating")) {
      toast("This session already exited; the live list has been refreshed.", "info");
      store.confirm = null;
      await refreshStore();
      return;
    }
    error.value = message;
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <dialog ref="dialog" class="modal-dialog confirm-dialog" aria-labelledby="confirm-title" @close="closeConfirm" @click="backdrop">
    <div v-if="request" class="modal-shell">
      <header class="modal-head">
        <div>
          <p class="eyebrow">Verified process action</p>
          <h2 id="confirm-title">{{ modeLabel }} this browser?</h2>
          <p>The backend will re-verify this session immediately before signaling anything.</p>
        </div>
        <button class="icon-btn" type="button" aria-label="Cancel action" :disabled="busy" @click="close">×</button>
      </header>
      <div class="modal-body">
        <dl class="confirm-facts">
          <div><dt>Browser</dt><dd>{{ request.session.browser }}</dd></div>
          <div><dt>Origin</dt><dd>{{ request.session.source.summary }}</dd></div>
          <div><dt>Root PID</dt><dd>{{ request.session.pid }}</dd></div>
          <div><dt>Processes</dt><dd>{{ request.session.process_count }} in verified subtree</dd></div>
          <div><dt>Signal</dt><dd>{{ verb }}</dd></div>
          <div><dt>Session</dt><dd><code>{{ request.session.id }}</code></dd></div>
        </dl>
        <p class="notice notice-warning">
          This sends a signal to the currently verified browser root and its same-UID descendants. It does not target the launching Agent or other Chrome processes. The action cannot be undone.
        </p>
        <label class="check-row" for="confirm-check">
          <input id="confirm-check" v-model="checked" type="checkbox" :disabled="busy" />
          <span>I understand the impact and want to {{ request.mode === "kill" ? "force kill" : "stop" }} this verified browser subtree.</span>
        </label>
        <p v-if="error" class="form-error" role="alert">{{ error }}</p>
      </div>
      <footer class="modal-actions">
        <button class="ghost-btn" type="button" :disabled="busy" @click="close">Cancel</button>
        <button
          class="danger-btn"
          type="button"
          :disabled="!checked || busy"
          @click="confirmAction"
        >
          {{ busy ? "Verifying…" : `${modeLabel} · ${verb}` }}
        </button>
      </footer>
    </div>
  </dialog>
</template>
