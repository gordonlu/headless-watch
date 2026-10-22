<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { downloadExport, getSettings, saveSettings } from "../api";
import type { Settings, SettingsPayload } from "../types";
import { fmtBytes } from "../format";
import { refreshStore, store, toast } from "../store";

const defaults: Settings = {
  interval_s: 1,
  cpu_alert_cores: 1,
  cpu_alert_duration_s: 30,
  leftover_after_s: 60,
  pss_interval_s: 15,
  cdp_interval_s: 5,
  notifications: true,
  global_mute: false,
  allow_screenshots: true,
  history_max: 360,
  retention_raw_hours: 24,
  retention_events_days: 30,
  cpu_clear_ratio: 0.75,
};

const form = reactive<Settings>({ ...defaults });
const original = ref(JSON.stringify(defaults));
const info = ref<SettingsPayload | null>(null);
const loading = ref(true);
const saving = ref(false);
const error = ref("");

const dirty = computed(() => JSON.stringify(form) !== original.value);

async function loadSettings() {
  loading.value = true;
  error.value = "";
  try {
    const payload = await getSettings();
    info.value = payload;
    Object.assign(form, payload.settings);
    original.value = JSON.stringify(payload.settings);
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Cannot load settings.";
  } finally {
    loading.value = false;
  }
}

async function save() {
  saving.value = true;
  error.value = "";
  try {
    const payload = await saveSettings({ ...form });
    info.value = payload;
    Object.assign(form, payload.settings);
    original.value = JSON.stringify(payload.settings);
    toast("Settings saved and applied to the live monitor.", "success");
    await refreshStore();
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Cannot save settings.";
  } finally {
    saving.value = false;
  }
}

function reset() {
  const settings = info.value?.settings || defaults;
  Object.assign(form, settings);
  original.value = JSON.stringify(settings);
}

async function exportFile(format: "json" | "csv", dataset = "sessions") {
  try {
    await downloadExport(format, dataset);
    toast(`Exported ${dataset} as ${format.toUpperCase()}.`, "success");
  } catch (err) {
    toast(err instanceof Error ? err.message : "Export failed.", "error");
  }
}

onMounted(loadSettings);
</script>

<template>
  <div class="page settings-page">
    <section class="section section-first">
      <div class="section-head">
        <div>
          <p class="eyebrow">Reliability controls</p>
          <h2>Settings</h2>
          <p class="section-sub">Sampling, alert thresholds, retention and privacy. Changes apply to the running monitor immediately.</p>
        </div>
        <div class="settings-actions">
          <button class="ghost-btn" type="button" :disabled="!dirty || saving" @click="reset">Discard</button>
          <button class="primary-btn" type="button" :disabled="!dirty || saving" @click="save">
            {{ saving ? "Saving…" : "Save settings" }}
          </button>
        </div>
      </div>

      <p v-if="error" class="form-error" role="alert">{{ error }}</p>
      <div v-if="loading" class="loading-panel">Loading configuration…</div>
      <div v-else class="settings-grid">
        <section class="panel settings-card">
          <div class="panel-head">
            <div><h2>Sampling</h2><p class="panel-sub">Fast process scans stay independent from PSS and CDP probes.</p></div>
          </div>
          <div class="form-grid">
            <label class="field">
              <span>Process scan interval</span>
              <input v-model.number="form.interval_s" type="number" min="0.5" max="10" step="0.5" />
              <small>0.5–10 seconds. Default 1s keeps discovery under 2s.</small>
            </label>
            <label class="field">
              <span>PSS interval</span>
              <input v-model.number="form.pss_interval_s" type="number" min="5" max="300" step="5" />
              <small>5–300 seconds. PSS never blocks the fast scanner.</small>
            </label>
            <label class="field">
              <span>CDP polling interval</span>
              <input v-model.number="form.cdp_interval_s" type="number" min="2" max="120" step="1" />
              <small>2–120 seconds for existing local endpoints.</small>
            </label>
            <label class="field">
              <span>Trend points retained</span>
              <input v-model.number="form.history_max" type="number" min="30" max="3600" step="30" />
              <small>In-memory points per chart and session sparkline.</small>
            </label>
          </div>
        </section>

        <section class="panel settings-card">
          <div class="panel-head">
            <div><h2>Alerts</h2><p class="panel-sub">CPU alerts require a sustained threshold, not a single spike.</p></div>
          </div>
          <div class="form-grid">
            <label class="field">
              <span>CPU threshold</span>
              <input v-model.number="form.cpu_alert_cores" type="number" min="0.05" max="1024" step="0.05" />
              <small>Logical cores. 1.0 means one fully busy CPU thread.</small>
            </label>
            <label class="field">
              <span>Sustained for</span>
              <input v-model.number="form.cpu_alert_duration_s" type="number" min="5" max="3600" step="5" />
              <small>Seconds above threshold before an alert event.</small>
            </label>
            <label class="field">
              <span>Suspected leftover delay</span>
              <input v-model.number="form.leftover_after_s" type="number" min="10" max="86400" step="10" />
              <small>Seconds after an observed launcher exit.</small>
            </label>
            <label class="field">
              <span>CPU clear ratio</span>
              <input v-model.number="form.cpu_clear_ratio" type="number" min="0.5" max="0.99" step="0.05" />
              <small>Drop below threshold × ratio before clearing the alert.</small>
            </label>
          </div>
          <div class="toggle-list">
            <label class="toggle-row">
              <span><strong>Desktop notifications</strong><small>Uses notify-send when a desktop notification service is available.</small></span>
              <input v-model="form.notifications" type="checkbox" />
            </label>
            <label class="toggle-row">
              <span><strong>Global mute</strong><small>Keeps recording alert events but suppresses desktop notifications.</small></span>
              <input v-model="form.global_mute" type="checkbox" />
            </label>
          </div>
          <p v-if="info && !info.notifications_available" class="notice notice-warning">notify-send was not found. Dashboard alerts still work; desktop delivery is unavailable.</p>
        </section>

        <section class="panel settings-card">
          <div class="panel-head">
            <div><h2>Privacy &amp; retention</h2><p class="panel-sub">Commands and URLs are redacted before storage. No screenshots are kept by default.</p></div>
          </div>
          <div class="form-grid">
            <label class="field">
              <span>Raw metric retention</span>
              <input v-model.number="form.retention_raw_hours" type="number" min="1" max="720" step="1" />
              <small>Hours of one-second storage in SQLite WAL.</small>
            </label>
            <label class="field">
              <span>Event retention</span>
              <input v-model.number="form.retention_events_days" type="number" min="1" max="365" step="1" />
              <small>Days for events, action audits and ended session metadata.</small>
            </label>
          </div>
          <label class="toggle-row">
            <span><strong>On-demand screenshots</strong><small>Allows the Inspector to capture a tab through an already-running local CDP endpoint. Captures are not stored on the server.</small></span>
            <input v-model="form.allow_screenshots" type="checkbox" />
          </label>
          <p class="notice">A screenshot can reveal logged-in page content. Use an isolated browser profile for test sessions.</p>
        </section>

        <section class="panel settings-card">
          <div class="panel-head">
            <div><h2>Runtime &amp; export</h2><p class="panel-sub">Diagnostics and local data packages always use redacted values.</p></div>
          </div>
          <dl class="fact-grid compact-facts">
            <div><dt>Version</dt><dd>{{ info?.version || store.overview?.version }}</dd></div>
            <div><dt>Database</dt><dd>{{ fmtBytes(info?.database_size ?? store.overview?.database_size) }}</dd></div>
            <div><dt>Data path</dt><dd class="mono wrap">{{ info?.data_path }}</dd></div>
            <div><dt>Monitor</dt><dd>{{ store.overview?.health.status }} · PID {{ store.overview?.health.monitor_pid }}</dd></div>
          </dl>
          <div class="export-row">
            <button class="ghost-btn small" type="button" @click="exportFile('json')">Export JSON</button>
            <button class="ghost-btn small" type="button" @click="exportFile('csv', 'sessions')">Sessions CSV</button>
            <button class="ghost-btn small" type="button" @click="exportFile('csv', 'events')">Events CSV</button>
            <button class="ghost-btn small" type="button" @click="exportFile('csv', 'actions')">Actions CSV</button>
          </div>
        </section>
      </div>
    </section>
  </div>
</template>
