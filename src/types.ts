export interface SessionUsage {
  window_start: number | null;
  reset_at: number | null;
  weighted_tokens: number;
  percent: number | null;
  calibrated: boolean;
  eta_secs: number | null;
  burn_rate_per_hour: number | null;
}

export interface WeeklyUsage {
  weighted_tokens: number;
  percent: number | null;
  reset_date: string | null;
  week_start: number | null;
  next_reset_at: number | null;
  calibrated: boolean;
}

export interface SessionCtx {
  session_id: string;
  pid: number;
  cwd: string;
  version: string;
  model: string | null;
  context_tokens: number;
  context_limit: number;
  percent: number | null;
  title: string | null;
  entrypoint: string | null;
  status: string | null;
  updated_at: number | null;
  weighted_5h: number;
}

export interface MemoryFile {
  project: string;
  name: string;
  path: string;
  content: string;
}

export interface RtkSavings {
  summary: {
    total_commands: number;
    total_input: number;
    total_output: number;
    total_saved: number;
    avg_savings_pct: number;
  };
  weekly: { week_start: string; saved_tokens: number; savings_pct: number }[];
}

export interface DayBucket {
  label: string;
  by_model: { model: string; weighted: number }[];
  is_today: boolean;
  cost_usd: number;
  breakdown: { input: number; output: number; cache_write: number };
}

export interface OutcomeCategory {
  kind: string;
  weighted: number;
  percent: number;
  session_count: number;
}

export interface OutcomeReport {
  window_start: number;
  window_end: number;
  categories: OutcomeCategory[];
}

export interface UpdateInfo {
  version: string;
  notes: string | null;
  url: string;
}

export interface Calibration {
  percent: number;
  budget: number;
  calibrated_at: number;
}

export interface Config {
  refresh_secs: number;
  weekly_reset_date: string | null;
  notifications_enabled: boolean;
  alert_levels: number[];
  tracking_enabled: boolean;
  session_calibration: Calibration | null;
  weekly_calibration: Calibration | null;
}

export interface PanelData {
  session: SessionUsage;
  weekly: WeeklyUsage;
  sessions: SessionCtx[];
  chart: DayBucket[];
  config: Config;
  update: UpdateInfo | null;
  rtk: RtkSavings | null;
  projects: string[];
}
