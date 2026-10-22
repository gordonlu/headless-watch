import type {
  EventRow,
  HistorySession,
  MetricPoint,
  Overview,
  Settings,
  SettingsPayload,
  TabsPayload,
  TerminationOutcome,
} from "./types";

const meta = document.querySelector<HTMLMetaElement>('meta[name="hw-token"]');
export const apiToken = meta?.content && meta.content !== "__TOKEN__" ? meta.content : "";

export class ApiError extends Error {
  status: number;

  constructor(message: string, status = 0) {
    super(message);
    this.status = status;
  }
}

async function parseError(response: Response): Promise<ApiError> {
  try {
    const payload = await response.json();
    return new ApiError(payload.error || response.statusText, response.status);
  } catch {
    return new ApiError(response.statusText || `HTTP ${response.status}`, response.status);
  }
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (apiToken) headers.set("X-Session-Token", apiToken);
  if (init.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  const response = await fetch(path, {
    ...init,
    headers,
    credentials: "same-origin",
    cache: "no-store",
  });
  if (!response.ok) throw await parseError(response);
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export function streamUrl(): string {
  return `/api/v1/stream?token=${encodeURIComponent(apiToken)}`;
}

export function getOverview(): Promise<Overview> {
  return request<Overview>("/api/v1/overview");
}

export function getHistory(params: {
  q?: string;
  source?: string;
  limit?: number;
} = {}): Promise<{ sessions: HistorySession[]; status: string }> {
  const search = new URLSearchParams({ status: "ended" });
  if (params.q) search.set("q", params.q);
  if (params.source) search.set("source", params.source);
  if (params.limit) search.set("limit", String(params.limit));
  return request(`/api/v1/sessions?${search.toString()}`);
}

export function getSession(id: string): Promise<unknown> {
  return request(`/api/v1/sessions/${encodeURIComponent(id)}`);
}

export function getMetrics(id: string, window = "30m"): Promise<{ points: MetricPoint[]; window_s: number }> {
  return request(
    `/api/v1/sessions/${encodeURIComponent(id)}/metrics?window=${encodeURIComponent(window)}`,
  );
}

export function getTabs(id: string): Promise<TabsPayload> {
  return request(`/api/v1/sessions/${encodeURIComponent(id)}/tabs`);
}

export function getEvents(sessionId?: string, limit = 80): Promise<{ events: EventRow[] }> {
  const search = new URLSearchParams({ limit: String(limit) });
  if (sessionId) search.set("session_id", sessionId);
  return request(`/api/v1/events?${search.toString()}`);
}

export function getActions(limit = 200): Promise<{ actions: import("./types").ActionRow[] }> {
  return request(`/api/v1/actions?limit=${limit}`);
}

export function getAlerts(limit = 200): Promise<{ alerts: EventRow[] }> {
  return request(`/api/v1/alerts?limit=${limit}`);
}

export function acknowledgeAlert(id: number): Promise<{ ok: boolean }> {
  return request(`/api/v1/alerts/${id}/ack`, { method: "POST" });
}

export function muteAlert(id: number, minutes = 30): Promise<{ ok: boolean }> {
  return request(`/api/v1/alerts/${id}/mute`, {
    method: "POST",
    body: JSON.stringify({ minutes }),
  });
}

export function muteSession(id: string, minutes = 30): Promise<{ ok: boolean }> {
  return request(`/api/v1/sessions/${encodeURIComponent(id)}/mute`, {
    method: "POST",
    body: JSON.stringify({ minutes }),
  });
}

export interface BatchTerminationResult {
  session_id: string;
  ok: boolean;
  result: string;
  message: string;
  targets?: number;
  signaled?: number;
  survivors?: number;
  force_available?: boolean;
}

export interface BatchTerminationOutcome {
  ok: boolean;
  mode: string;
  count: number;
  results: BatchTerminationResult[];
}

export function terminateBatch(
  sessions: { id: string; identity: { pid: number; start_ticks: number } }[],
  mode: "term" | "kill" = "term",
): Promise<BatchTerminationOutcome> {
  return request("/api/v1/terminate", {
    method: "POST",
    body: JSON.stringify({ sessions, mode, confirm: true }),
  });
}

export async function terminateSession(
  id: string,
  identity: { pid: number; start_ticks: number },
  mode: "term" | "kill",
): Promise<TerminationOutcome> {
  const response = await fetch(`/api/v1/sessions/${encodeURIComponent(id)}/terminate`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Session-Token": apiToken,
    },
    body: JSON.stringify({ identity, mode, confirm: true }),
    credentials: "same-origin",
    cache: "no-store",
  });
  const payload = await response.json().catch(() => ({}));
  if (response.ok || (response.status === 409 && payload.result)) {
    return payload as TerminationOutcome;
  }
  throw new ApiError(payload.error || response.statusText, response.status);
}

export async function screenshotSession(id: string, tabId: string): Promise<Blob> {
  const headers = new Headers({
    "Content-Type": "application/json",
    "X-Session-Token": apiToken,
  });
  const response = await fetch(`/api/v1/sessions/${encodeURIComponent(id)}/screenshot`, {
    method: "POST",
    headers,
    body: JSON.stringify({ tab_id: tabId }),
    credentials: "same-origin",
    cache: "no-store",
  });
  if (!response.ok) throw await parseError(response);
  return response.blob();
}

export function getSettings(): Promise<SettingsPayload> {
  return request("/api/v1/settings");
}

export function saveSettings(settings: Settings): Promise<SettingsPayload> {
  return request("/api/v1/settings", {
    method: "PATCH",
    body: JSON.stringify(settings),
  });
}

export async function downloadExport(
  format: "json" | "csv",
  dataset = "sessions",
): Promise<void> {
  const headers = new Headers({ "X-Session-Token": apiToken });
  const response = await fetch(
    `/api/v1/export?format=${encodeURIComponent(format)}&dataset=${encodeURIComponent(dataset)}`,
    { headers, credentials: "same-origin", cache: "no-store" },
  );
  if (!response.ok) throw await parseError(response);
  const blob = await response.blob();
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `headless-watch-${dataset}.${format}`;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
}
