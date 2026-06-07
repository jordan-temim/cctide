export interface SessionUsage {
  window_start: number | null;
  reset_at: number | null;
  weighted_tokens: number;
  percent: number | null;
  calibrated: boolean;
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
  cwd: string;
  version: string;
  model: string | null;
  context_tokens: number;
  context_limit: number;
  percent: number | null;
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
  session_calibration_2: Calibration | null;
  weekly_calibration: Calibration | null;
  weekly_calibration_2: Calibration | null;
}

export interface PanelData {
  session: SessionUsage;
  weekly: WeeklyUsage;
  sessions: SessionCtx[];
  chart: DayBucket[];
  config: Config;
  update: UpdateInfo | null;
  rtk: RtkSavings | null;
}
