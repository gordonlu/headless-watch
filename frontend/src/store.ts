import { computed, reactive } from "vue";
import * as api from "./api";
import type { Overview, SessionView } from "./types";

export type Theme = "default" | "terminal";

export interface Toast {
  id: number;
  message: string;
  kind: "info" | "success" | "warning" | "error";
}

export interface ConfirmRequest {
  session: SessionView;
  mode: "term" | "kill";
}

export const store = reactive({
  overview: null as Overview | null,
  theme: "default" as Theme,
  loading: true,
  connected: false,
  error: "",
  nowMs: Date.now(),
  toasts: [] as Toast[],
  inspectorId: null as string | null,
  pendingInspectorId: null as string | null,
  pendingHistoryId: null as string | null,
  confirm: null as ConfirmRequest | null,
  helpOpen: false,
});

export const sessions = computed<SessionView[]>(() => store.overview?.sessions ?? []);
export const selectedSession = computed<SessionView | null>(() =>
  store.inspectorId
    ? sessions.value.find((session) => session.id === store.inspectorId) ?? null
    : null,
);
export const recentEnded = computed(() => store.overview?.recent_ended ?? []);
export const isWindows = computed(() => store.overview?.health.platform === "windows");
export const pssSupported = computed(() => store.overview?.health.pss_supported !== false);
export const gracefulStopSupported = computed(
  () => store.overview?.health.graceful_stop_supported ?? !isWindows.value,
);
export const basicMemoryLabel = computed(() => isWindows.value ? "Working Set" : "RSS");
export const stopLabel = computed(() => gracefulStopSupported.value ? "Stop" : "Force stop");
export const stopMode = computed<"term" | "kill">(() =>
  gracefulStopSupported.value ? "term" : "kill",
);
export const pendingAlerts = computed(() =>
  (store.overview?.events ?? []).filter(
    (event) =>
      ["high_cpu", "suspected_leftover", "action_failed"].includes(event.type) &&
      !event.acknowledged &&
      (!event.muted_until || event.muted_until <= Date.now() / 1000),
  ),
);

let initialized = false;
let stream: EventSource | null = null;
let toastId = 0;

const apiMessage = (error: unknown): string =>
  error instanceof Error ? error.message : "Unexpected request failure.";

export async function refreshStore(): Promise<void> {
  try {
    store.overview = await api.getOverview();
    store.error = "";
  } catch (error) {
    store.error = apiMessage(error);
  } finally {
    store.loading = false;
  }
}

function connectStream(): void {
  if (stream) stream.close();
  stream = new EventSource(api.streamUrl());
  stream.onopen = () => {
    store.connected = true;
  };
  stream.onerror = () => {
    store.connected = false;
  };
  stream.addEventListener("state", (event) => {
    try {
      store.overview = JSON.parse((event as MessageEvent).data) as Overview;
      store.connected = true;
      store.error = "";
      store.loading = false;
    } catch {
      store.error = "The monitor sent an unreadable snapshot.";
    }
  });
}

export async function initStore(): Promise<void> {
  if (initialized) return;
  initialized = true;
  window.setInterval(() => {
    store.nowMs = Date.now();
  }, 1000);
  window.setInterval(() => {
    if (!store.connected && document.visibilityState === "visible") {
      void refreshStore();
    }
  }, 5000);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") void refreshStore();
  });
  await refreshStore();
  connectStream();
}

export function toast(message: string, kind: Toast["kind"] = "info"): void {
  const id = ++toastId;
  store.toasts.push({ id, message, kind });
  window.setTimeout(() => {
    const index = store.toasts.findIndex((item) => item.id === id);
    if (index >= 0) store.toasts.splice(index, 1);
  }, 4200);
}

export function dismissToast(id: number): void {
  const index = store.toasts.findIndex((item) => item.id === id);
  if (index >= 0) store.toasts.splice(index, 1);
}

export function openInspector(id: string): void {
  store.inspectorId = id;
}

export function closeInspector(): void {
  store.inspectorId = null;
}

export function requestConfirm(session: SessionView, mode: "term" | "kill"): void {
  store.confirm = { session, mode: gracefulStopSupported.value ? mode : "kill" };
}

export function closeConfirm(): void {
  store.confirm = null;
}

export async function copyText(value: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(value);
    toast("Diagnostic report copied.", "success");
  } catch {
    toast("Clipboard access was refused by the browser.", "error");
  }
}

export function diagnosticReport(session: SessionView): string {
  return JSON.stringify(
    {
      generated_at: new Date().toISOString(),
      id: session.id,
      identity: session.identity,
      browser: session.browser,
      source: session.source,
      status: session.status,
      cpu: session.cpu,
      memory: session.memory,
      parent: session.parent,
      ancestry: session.ancestry,
      processes: session.processes,
      cdp: { ...session.cdp, tabs: session.cdp.tabs.map((tab) => ({ id: tab.id, title: tab.title, url: tab.url })) },
    },
    null,
    2,
  );
}
