<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import { store } from "../store";

const dialog = ref<HTMLDialogElement | null>(null);

watch(
  () => store.helpOpen,
  async (open) => {
    await nextTick();
    if (!dialog.value) return;
    if (open && !dialog.value.open) dialog.value.showModal();
    if (!open && dialog.value.open) dialog.value.close();
  },
);

function close() {
  store.helpOpen = false;
}

function backdrop(event: MouseEvent) {
  if (event.target === dialog.value) close();
}
</script>

<template>
  <dialog ref="dialog" class="modal-dialog" aria-labelledby="help-title" @close="close" @click="backdrop">
    <div class="modal-shell">
      <header class="modal-head">
        <div>
          <p class="eyebrow">Local shortcuts</p>
          <h2 id="help-title">Keyboard</h2>
        </div>
        <button class="icon-btn" type="button" aria-label="Close shortcuts" @click="close">×</button>
      </header>
      <div class="modal-body">
        <dl class="shortcut-list">
          <div><dt><kbd>/</kbd></dt><dd>Focus session search</dd></div>
          <div><dt><kbd>Esc</kbd></dt><dd>Close Inspector, dialogs and this panel</dd></div>
          <div><dt><kbd>?</kbd></dt><dd>Open this shortcut list</dd></div>
        </dl>
        <p class="fine-print">Headless Watch never executes arbitrary CDP JavaScript, navigates pages, or sends input events.</p>
      </div>
      <footer class="modal-actions">
        <button class="ghost-btn" type="button" @click="close">Close</button>
      </footer>
    </div>
  </dialog>
</template>
