// Client for the `fusionsim serve` API (toksym-compatible endpoints plus
// model selection and the live model used by the control panel).

export interface ApiInfo {
  dt: number
  eval_shots: number
  eval_seed: number
  python: string
  models_dir: string
  extras: string[]
}

export interface LocalModel {
  id: string
  folder: string | null
  name: string | null
  notes: string | null
  entry: string | null
  files: number
  problems: string[]
}

export interface UploadedModel {
  dir: string
  root: string | null
  manifest: { entry?: string; files?: string[]; name?: string; notes?: string; requires?: string[] } | null
  entry: string | null
  files: string[]
  problems: string[]
  suggestion: string | null
}

export interface JobProgress<T> {
  running: boolean
  done: number
  total: number
  error: string | null
  result: T | null
}

export interface DatasetResult {
  dir: string
  shots: number
  disrupted: number
  rows: number
  columns: number
}

import type { CauseInfo, Mechanism, Shares } from './mechanisms'

export type Verdict = 'caught' | 'late' | 'missed' | 'false_alarm' | 'quiet' | 'excluded'

export interface ShotReport {
  shot_id: number
  scenario: string
  disrupted: boolean
  t_tq: number | null
  t_end: number | null
  n_steps: number
  spontaneous: boolean
  wall_contact: boolean
  t_alarm: number | null
  warning_s: number | null
  verdict: Verdict
  staircase: [number, number][]
  /** The engine's split of λ at the trigger (disrupted shots). */
  true_cause?: CauseInfo | null
  /** The model's split where its λ peaked — its guess of the cause. */
  predicted_cause?: CauseInfo | null
  /** The model's split at the alarm moment. */
  alarm_cause?: CauseInfo | null
}

export interface MechanismSummary {
  n: number
  agree: number
  accuracy: number
  confusion: Partial<Record<Mechanism, Partial<Record<Mechanism, number>>>>
  mean_true_share_pct: Shares
  mean_predicted_share_pct: Shares
}

export interface LiveInfo {
  p?: number
  p_now?: number
  risk_total?: number
  share_pct?: Shares
  drivers?: { fgw?: number; q95?: number; frad?: number; beta_n?: number }
  /** Fixer student: chosen action and the cost of every action */
  action?: string
  costs?: Record<string, number>
}

export interface RunReport {
  model: string
  run: { shots: number; seed: number; shift?: boolean }
  summary: {
    threshold: number
    tpr: number
    fpr: number
    tp: number
    fn: number
    fp: number
    tn: number
    late_alarms: number
    median_warning_s: number | null
    n_disruptions_scored: number
    n_quiet: number
    threshold_source: string
    n_shots: number
    n_disrupted: number
    n_spontaneous: number
    n_wall_contact: number
    mechanisms?: MechanismSummary | null
  }
  rule: { lead_time_s: number; fpr_budget: number; threshold: number | null }
  timing: { latency_p50_ms: number; latency_p99_ms: number; latency_max_ms: number; over_budget_steps: number; steps: number }
  shots: ShotReport[]
}

async function call<T>(url: string, body?: unknown): Promise<T> {
  const r = await fetch(url, body === undefined ? undefined : {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  const j = await r.json().catch(() => null)
  if (!r.ok) throw new Error(j?.error ?? `${url}: ${r.status}`)
  return j as T
}

export const api = {
  info: () => call<ApiInfo>('/api/info'),
  models: () => call<{ dir: string; python: string; models: LocalModel[] }>('/api/models'),
  dataset: (req: { shots: number; seed: number; stride: number; extras: string[] }) => call('/api/dataset', req),
  datasetProgress: () => call<JobProgress<DatasetResult>>('/api/dataset/progress'),
  datasetCancel: () => call('/api/dataset/cancel', {}),
  datasetFile: (name: 'measured.csv' | 'shots.csv') => `/api/dataset/file?name=${name}`,
  validate: (req: { model: string; lead_time_s: number; fpr_budget: number; shots?: number; seed?: number }) =>
    call('/api/validate', req),
  validateProgress: () => call<JobProgress<RunReport>>('/api/validate/progress'),
  validateCancel: () => call('/api/validate/cancel', {}),
  validateFile: (name: 'measured.csv' | 'shots.csv' | 'run.json') => `/api/validate/file?name=${name}`,
  rescore: (report: RunReport, lead_time_s: number, fpr_budget: number) =>
    call<RunReport>('/api/rescore', { report, lead_time_s, fpr_budget }),
  modelReset: () => call('/api/model/reset', {}),
  modelList: () => call<UploadedModel>('/api/model/list'),
  async upload(files: { file: File; path: string }[], onProgress?: (n: number, total: number) => void) {
    await call('/api/model/reset', {})
    let n = 0
    for (const { file, path } of files) {
      const r = await fetch(`/api/model/file?path=${encodeURIComponent(path)}`, { method: 'POST', body: file })
      if (!r.ok) {
        const j = await r.json().catch(() => null)
        throw new Error(j?.error ?? `${path}: ${r.status}`)
      }
      onProgress?.(++n, files.length)
    }
    return call<UploadedModel>('/api/model/list')
  },
  liveStart: (model: string, duration: number, scenario = 'interactive') =>
    call<{ ok: boolean; model: string | null }>('/api/live/start', { model, duration, scenario }),
  liveStep: (steps: { t: number; v: number[]; extra?: unknown }[]) => call<{ p: number[]; info?: (LiveInfo | null)[] }>('/api/live/step', { steps }),
  liveShot: (duration: number, scenario = 'interactive') => call('/api/live/shot', { duration, scenario }),
  liveStop: () => call('/api/live/stop', {}),
}

/** Files from a dropped folder (DataTransferItemList), with relative paths. */
export async function filesFromDrop(items: DataTransferItemList): Promise<{ file: File; path: string }[]> {
  const entries: FileSystemEntry[] = []
  for (const it of Array.from(items)) {
    const e = it.webkitGetAsEntry?.()
    if (e) entries.push(e)
  }
  const out: { file: File; path: string }[] = []
  const walk = async (e: FileSystemEntry): Promise<void> => {
    if (e.isFile) {
      const f = await new Promise<File>((res, rej) => (e as FileSystemFileEntry).file(res, rej))
      out.push({ file: f, path: e.fullPath.replace(/^\/+/, '') })
      return
    }
    if (!e.isDirectory) return
    const reader = (e as FileSystemDirectoryEntry).createReader()
    for (;;) {
      const batch = await new Promise<FileSystemEntry[]>((res, rej) => reader.readEntries(res, rej))
      if (batch.length === 0) break
      for (const b of batch) await walk(b)
    }
  }
  for (const e of entries) await walk(e)
  return out
}

export function filesFromInput(list: FileList): { file: File; path: string }[] {
  return Array.from(list).map((f) => ({ file: f, path: f.webkitRelativePath || f.name }))
}

export const pct = (x: number | null | undefined) => (typeof x === 'number' ? `${(x * 100).toFixed(1)}%` : '—')
