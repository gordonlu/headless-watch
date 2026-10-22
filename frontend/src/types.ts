export interface ProcessIdentity {
  boot_id: string;
  pid: number;
  start_ticks: number;
}

export interface CpuView {
  cores: number | null;
  single_core_pct: number | null;
  machine_pct: number | null;
  peak_cores: number | null;
  quality: string;
  online_cpus: number;
}

export interface PssCoverage {
  sampled: number;
  total: number;
}

export interface MemoryView {
  rss_sum: number;
  rss_quality: string;
  pss_sum: number | null;
  pss_quality: string;
  pss_age_s: number | null;
  pss_coverage: PssCoverage | null;
  pss_error: string | null;
}

export interface StatusView {
  lifecycle: "active" | "ended" | string;
  stale: boolean;
  parent_exited: boolean;
  parent_state: string;
  launcher_exited: boolean;
  launcher_exited_at: number | null;
  driver_exited: boolean;
  driver_exited_at: number | null;
  watch_exited_at: number | null;
  suspected_leftover: boolean;
  suspected_at: number | null;
  high_cpu: boolean;
  high_cpu_at: number | null;
  terminating: boolean;
  risk: "none" | "info" | "warning" | string;
  muted_until: number | null;
}

export interface SourceView {
  summary: string;
  confidence: string;
  launcher: string;
  launcher_confidence: string;
  launcher_pid: number | null;
  launcher_name: string | null;
  driver: string;
  driver_confidence: string;
  driver_pid: number | null;
  driver_name: string | null;
}

export interface AncestorView {
  pid: number;
  ppid: number;
  name: string;
  command: string;
  role: string | null;
  start_ticks: number;
  alive: boolean;
}

export interface ParentView {
  pid: number | null;
  name: string | null;
  observable: boolean;
  state: string;
  exited: boolean;
  exited_at: number | null;
  note: string | null;
}

export interface ProcessView {
  pid: number;
  ppid: number;
  name: string;
  role: string;
  command: string;
  uid: number | null;
  rss_bytes: number;
}

export interface CdpTab {
  id: string;
  title: string;
  url: string;
}

export interface CdpView {
  mode: "tcp" | "pipe" | "none" | string;
  state: string;
  port: number | null;
  reason: string | null;
  tabs: CdpTab[];
  screenshot_available: boolean;
}

export interface HistoryPoint {
  t: number;
  cores: number;
}

export interface SessionView {
  id: string;
  pid: number;
  start_ticks: number;
  identity: ProcessIdentity;
  browser: string;
  executable: string;
  command: string;
  cgroup: string | null;
  uid: number | null;
  discovery_mode: string;
  preexisting: boolean;
  first_seen_at: number;
  launched_at: number;
  ended_at: number | null;
  lifetime_s: number;
  source: SourceView;
  ancestry: AncestorView[];
  parent: ParentView;
  status: StatusView;
  cpu: CpuView;
  memory: MemoryView;
  process_count: number;
  processes: ProcessView[];
  roles: Record<string, number>;
  history: HistoryPoint[];
  cdp: CdpView;
  can_terminate: boolean;
  force_available: boolean;
  action_notice: string | null;
}

export interface SummaryView {
  active_sessions: number;
  process_count: number;
  cpu_cores: number;
  cpu_single_core_pct: number;
  cpu_machine_pct: number;
  online_cpus: number;
  calculating_sessions: number;
  rss_sum: number;
  pss_sum: number | null;
  pss_age_s: number | null;
  parent_exited: number;
  suspected_leftovers: number;
  high_cpu: number;
  unknown_origin: number;
}

export interface HealthView {
  status: string;
  last_scan_age_s: number | null;
  last_scan_at: string | null;
  scan_failures: number;
  last_error: string | null;
  stale_sessions: number;
  online_cpus: number;
  monitor_pid: number;
  active_sessions: number;
}

export interface SystemPoint {
  t: number;
  cores: number;
  machine_pct: number;
  rss_sum: number;
  pss_sum: number | null;
}

export interface EventRow {
  id: number;
  t: number;
  at: string | null;
  session_id: string | null;
  type: string;
  severity: string;
  message: string;
  evidence: Record<string, unknown>;
  acknowledged: boolean;
  muted_until: number | null;
}

export interface ActionRow {
  id: number;
  t: number;
  at: string | null;
  session_id: string;
  action: string;
  targets: number;
  result: string;
  message: string;
  evidence: Record<string, unknown>;
}

export interface Settings {
  interval_s: number;
  cpu_alert_cores: number;
  cpu_alert_duration_s: number;
  leftover_after_s: number;
  pss_interval_s: number;
  cdp_interval_s: number;
  notifications: boolean;
  global_mute: boolean;
  allow_screenshots: boolean;
  history_max: number;
  retention_raw_hours: number;
  retention_events_days: number;
  cpu_clear_ratio: number;
}

export interface RecentEnded {
  id: string;
  pid: number;
  browser: string;
  executable: string;
  discovery_mode: string;
  launched_at: number;
  ended_at: number | null;
  peak_cpu: number | null;
  latest_rss: number;
  latest_pss: number | null;
  latest_process_count: number;
  source: SourceView | Record<string, unknown>;
  status: StatusView | Record<string, unknown>;
}

export interface Overview {
  version: string;
  generated_at: number;
  uptime_s: number;
  revision: number;
  started_at: string;
  health: HealthView;
  summary: SummaryView;
  sessions: SessionView[];
  recent_ended: RecentEnded[];
  events: EventRow[];
  actions: ActionRow[];
  system_history: SystemPoint[];
  settings: Settings;
  database_size: number;
}

export interface HistorySession {
  id: string;
  boot_id: string;
  root_pid: number;
  root_start_ticks: number;
  browser: string;
  executable: string;
  command: string;
  cgroup: string | null;
  uid: number | null;
  launched_at: number;
  first_seen_at: number;
  last_seen_at: number;
  ended_at: number | null;
  discovery_mode: string;
  source: SourceView | Record<string, unknown>;
  ancestry: AncestorView[] | Record<string, unknown>[];
  parent: ParentView | Record<string, unknown>;
  status: StatusView | Record<string, unknown>;
  peak_cpu: number | null;
  latest_rss: number;
  latest_pss: number | null;
  latest_process_count: number;
  final_view: SessionView | null;
}

export interface MetricPoint {
  t: number;
  cpu_cores: number;
  rss_sum: number;
  pss_sum: number | null;
  process_count: number;
  quality: string;
}

export interface SettingsPayload {
  settings: Settings;
  notifications_available: boolean;
  database_size: number;
  version: string;
  data_path: string;
}

export interface TabsPayload {
  session_id: string;
  mode: string;
  state: string;
  port: number | null;
  reason: string | null;
  tabs: CdpTab[];
}

export interface TerminationOutcome {
  ok: boolean;
  result: string;
  message: string;
  mode: string;
  targets: number;
  signaled: number;
  survivors: number;
  unauthorized: number;
  skipped: number;
  force_available: boolean;
  details: Record<string, unknown>;
}

export type PageName = "overview" | "live" | "history" | "alerts" | "settings";
