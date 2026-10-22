<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import type { EventRow } from "../types";
import { getEvents, muteSession, screenshotSession } from "../api";
import {
  closeInspector,
  copyText,
  diagnosticReport,
  requestConfirm,
  selectedSession,
  store,
  toast,
} from "../store";
import { fmtBytes, fmtCores, fmtDateTime, fmtDuration, fmtPct } from "../format";
import StatusBadges from "./StatusBadges.vue";
import ProcessTree from "./ProcessTree.vue";
import EventTimeline from "./EventTimeline.vue";

type Tab = "overview" | "processes" | "browser" | "events";
const tab = ref<Tab>("overview");
const events = ref<EventRow[]>([]);
const eventsLoading = ref(false);
const closeButton = ref<HTMLButtonElement | null>(null);
const screenshotDialog = ref<HTMLDialogElement | null>(null);
const screenshotUrl = ref("");
const screenshotLoading = ref(false);
const screenshotError = ref("");
const screenshotTitle = ref("");

const session = computed(() => selectedSession.value);

watch(
  () => session.value?.id,
  async (id, previous) => {
    if (!id) {
      document.body.classList.remove("has-drawer");
      closeScreenshot();
      return;
    }
    document.body.classList.add("has-drawer");
    if (!previous) {
      tab.value = "overview";
      events.value = [];
      await nextTick();
      closeButton.value?.focus();
    }
  },
  { immediate: true },
);

watch(tab, (value) => {
  if (value === "events" && session.value && !eventsLoading.value) void loadEvents(session.value.id);
});

watch(
  () => session.value?.id,
  () => {
    if (tab.value === "events" && session.value) void loadEvents(session.value.id);
  },
);

onBeforeUnmount(() => {
  document.body.classList.remove("has-drawer");
  revokeScreenshot();
});

async function loadEvents(id: string) {
  eventsLoading.value = true;
  try {
    const response = await getEvents(id, 120);
    events.value = response.events;
  } catch (err) {
    toast(err instanceof Error ? err.message : "Cannot load session events.", "error");
  } finally {
    eventsLoading.value = false;
  }
}

function revokeScreenshot() {
  if (screenshotUrl.value) {
    URL.revokeObjectURL(screenshotUrl.value);
    screenshotUrl.value = "";
  }
}

async function openScreenshot(tabId: string, title: string) {
  if (!session.value) return;
  revokeScreenshot();
  screenshotTitle.value = title;
  screenshotError.value = "";
  screenshotLoading.value = true;
  if (screenshotDialog.value && !screenshotDialog.value.open) screenshotDialog.value.showModal();
  try {
    const blob = await screenshotSession(session.value.id, tabId);
    screenshotUrl.value = URL.createObjectURL(blob);
  } catch (err) {
    screenshotError.value = err instanceof Error ? err.message : "Screenshot failed.";
  } finally {
    screenshotLoading.value = false;
  }
}

function closeScreenshot() {
  if (screenshotDialog.value?.open) screenshotDialog.value.close();
  revokeScreenshot();
  screenshotError.value = "";
}

function screenshotBackdrop(event: MouseEvent) {
  if (event.target === screenshotDialog.value) closeScreenshot();
}

async function mute() {
  if (!session.value) return;
  try {
    await muteSession(session.value.id, 30);
    toast("Session notifications muted for 30 minutes.", "success");
  } catch (err) {
    toast(err instanceof Error ? err.message : "Cannot mute session.", "error");
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition name="drawer">
      <div v-if="session" class="inspector-backdrop" @click.self="closeInspector">
        <aside class="inspector" role="dialog" aria-modal="true" aria-labelledby="inspector-title">
          <header class="inspector-head">
            <div class="inspector-title-wrap">
              <div class="browser-mark" aria-hidden="true">CH</div>
              <div>
                <p class="eyebrow">{{ session.source.confidence }} source evidence</p>
                <h2 id="inspector-title">{{ session.browser }}</h2>
                <p class="inspector-sub">PID {{ session.pid }} · {{ session.source.summary }} · {{ fmtDuration(session.lifetime_s) }}</p>
              </div>
            </div>
            <button ref="closeButton" class="icon-btn" type="button" aria-label="Close Inspector" @click="closeInspector">×</button>
          </header>

          <div class="inspector-badges">
            <StatusBadges :session="session" />
          </div>

          <nav class="inspector-tabs" aria-label="Session detail sections">
            <button class="tab-button" :class="{ 'is-active': tab === 'overview' }" type="button" @click="tab = 'overview'">Overview</button>
            <button class="tab-button" :class="{ 'is-active': tab === 'processes' }" type="button" @click="tab = 'processes'">Process tree</button>
            <button class="tab-button" :class="{ 'is-active': tab === 'browser' }" type="button" @click="tab = 'browser'">Browser</button>
            <button class="tab-button" :class="{ 'is-active': tab === 'events' }" type="button" @click="tab = 'events'">Events</button>
          </nav>

          <div class="inspector-body">
            <div v-if="tab === 'overview'" class="inspector-section">
              <div class="detail-metrics">
                <div class="detail-metric">
                  <span>CPU</span>
                  <strong>{{ session.cpu.cores === null ? "Calculating" : `${fmtCores(session.cpu.cores)} cores` }}</strong>
                  <small>{{ fmtPct(session.cpu.single_core_pct) }} one core · {{ fmtPct(session.cpu.machine_pct) }} machine</small>
                </div>
                <div class="detail-metric">
                  <span>Memory</span>
                  <strong>{{ fmtBytes(session.memory.pss_sum ?? session.memory.rss_sum) }}</strong>
                  <small>{{ session.memory.pss_sum !== null ? "PSS" : "RSS upper bound" }} · {{ session.process_count }} processes</small>
                </div>
                <div class="detail-metric">
                  <span>Peak CPU</span>
                  <strong>{{ fmtCores(session.cpu.peak_cores) }} cores</strong>
                  <small>observed in this run</small>
                </div>
                <div class="detail-metric">
                  <span>Runtime</span>
                  <strong>{{ fmtDuration(session.lifetime_s) }}</strong>
                  <small>{{ session.discovery_mode }}</small>
                </div>
              </div>

              <h3>Evidence</h3>
              <dl class="fact-grid">
                <div><dt>Origin</dt><dd>{{ session.source.summary }} · {{ session.source.confidence }}</dd></div>
                <div><dt>Launcher</dt><dd>{{ session.source.launcher }} <span class="mono">{{ session.source.launcher_pid ?? "—" }}</span></dd></div>
                <div><dt>Browser driver</dt><dd>{{ session.source.driver }} <span class="mono">{{ session.source.driver_pid ?? "—" }}</span></dd></div>
                <div><dt>Direct parent</dt><dd>{{ session.parent.name || "Not observable" }} <span class="mono">{{ session.parent.pid ?? "—" }}</span></dd></div>
                <div><dt>First seen</dt><dd>{{ fmtDateTime(session.first_seen_at) }}</dd></div>
                <div><dt>Launched</dt><dd>{{ fmtDateTime(session.launched_at) }}</dd></div>
                <div><dt>Exit watch</dt><dd>{{ session.status.watch_exited_at ? fmtDateTime(session.status.watch_exited_at) : "No observed exit" }}</dd></div>
                <div><dt>CDP</dt><dd>{{ session.cdp.mode === "tcp" ? `TCP :${session.cdp.port} · ${session.cdp.tabs.length} tabs` : session.cdp.mode === "pipe" ? "Pipe only" : "No endpoint" }}</dd></div>
                <div class="fact-wide"><dt>Executable</dt><dd class="mono wrap">{{ session.executable }}</dd></div>
                <div class="fact-wide"><dt>Command</dt><dd class="mono wrap">{{ session.command }}</dd></div>
                <div class="fact-wide"><dt>cgroup</dt><dd class="mono wrap">{{ session.cgroup || "Not available" }}</dd></div>
              </dl>

              <p v-if="session.parent.note" class="notice notice-info">{{ session.parent.note }}</p>
              <p v-if="session.action_notice" class="notice notice-warning">{{ session.action_notice }}</p>
              <div class="role-row">
                <span v-for="(count, role) in session.roles" :key="role" class="role-chip">{{ role }} <strong>{{ count }}</strong></span>
              </div>
            </div>

            <ProcessTree
              v-else-if="tab === 'processes'"
              :ancestry="session.ancestry"
              :processes="session.processes"
              :root-pid="session.pid"
              :confidence="session.source.confidence"
            />

            <div v-else-if="tab === 'browser'" class="inspector-section">
              <div class="cdp-banner" :class="`cdp-${session.cdp.state}`">
                <strong>{{ session.cdp.state === "available" ? "CDP available" : session.cdp.state === "pipe_only" ? "CDP pipe only" : "CDP unavailable" }}</strong>
                <span>{{ session.cdp.reason || `Local endpoint on port ${session.cdp.port}.` }}</span>
              </div>
              <div v-if="session.cdp.tabs.length" class="tab-list">
                <article v-for="item in session.cdp.tabs" :key="item.id" class="browser-tab">
                  <div>
                    <h3>{{ item.title || "Untitled page" }}</h3>
                    <p class="mono wrap">{{ item.url }}</p>
                  </div>
                  <button
                    class="ghost-btn small"
                    type="button"
                    :disabled="!session.cdp.screenshot_available"
                    @click="openScreenshot(item.id, item.title || 'Untitled page')"
                  >Screenshot</button>
                </article>
              </div>
              <div v-else class="empty-inline">
                {{ session.cdp.mode === "tcp" ? "No page targets were reported by the local endpoint." : "Start Chrome with an existing TCP debugging endpoint to view tabs and request screenshots." }}
              </div>
              <p class="fine-print">Screenshots are captured once, on request. Headless Watch never records a continuous stream, executes page JavaScript, or sends input events.</p>
            </div>

            <div v-else class="inspector-section">
              <div class="event-head">
                <h3>Session events</h3>
                <button class="ghost-btn small" type="button" @click="session && loadEvents(session.id)">Refresh</button>
              </div>
              <p v-if="eventsLoading" class="loading-inline">Loading events…</p>
              <EventTimeline v-else :events="events" :limit="120" />
            </div>
          </div>

          <footer class="inspector-footer">
            <div class="footer-left">
              <button class="ghost-btn small" type="button" @click="copyText(diagnosticReport(session))">Copy report</button>
              <button class="ghost-btn small" type="button" @click="mute">Mute 30m</button>
            </div>
            <div class="footer-actions">
              <button
                v-if="session.force_available"
                class="danger-btn small"
                type="button"
                @click="requestConfirm(session, 'kill')"
              >Force kill · SIGKILL</button>
              <button
                class="stop-btn"
                type="button"
                :disabled="!session.can_terminate"
                @click="requestConfirm(session, 'term')"
              >Stop · SIGTERM</button>
            </div>
          </footer>
        </aside>
      </div>
    </Transition>

    <dialog ref="screenshotDialog" class="modal-dialog screenshot-dialog" aria-labelledby="screenshot-title" @close="closeScreenshot" @click="screenshotBackdrop">
      <div class="modal-shell">
        <header class="modal-head">
          <div>
            <p class="eyebrow">On-demand capture</p>
            <h2 id="screenshot-title">{{ screenshotTitle || "Tab screenshot" }}</h2>
            <p>Captured once through the browser's existing local CDP endpoint.</p>
          </div>
          <button class="icon-btn" type="button" aria-label="Close screenshot" @click="closeScreenshot">×</button>
        </header>
        <div class="modal-body screenshot-body">
          <p v-if="screenshotLoading" class="loading-inline">Capturing from the local endpoint…</p>
          <p v-else-if="screenshotError" class="form-error">{{ screenshotError }}</p>
          <img v-else-if="screenshotUrl" :src="screenshotUrl" alt="Screenshot of the selected browser tab" />
        </div>
        <footer class="modal-actions">
          <span class="fine-print">Not saved by the backend. Avoid sharing captures of logged-in pages.</span>
          <button class="ghost-btn" type="button" @click="closeScreenshot">Close</button>
        </footer>
      </div>
    </dialog>
  </Teleport>
</template>
