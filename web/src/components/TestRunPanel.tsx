import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  api, filesFromDrop, filesFromInput, pct,
  type LocalModel, type MechanismSummary, type RunReport, type ShotReport, type UploadedModel, type Verdict,
} from '../lib/api'
import { MECHANISMS, MECH_INFO, dominantOf, type Shares } from '../lib/mechanisms'

const VERDICTS: { key: Verdict; label: string; color: string; hint: string }[] = [
  { key: 'caught', label: 'Спіймано', color: '#34d399', hint: 'Тривога щонайменше за Lead time до термічного гасіння' },
  { key: 'late', label: 'Запізно', color: '#fbbf24', hint: 'Тривога була, але надто близько до гасіння' },
  { key: 'missed', label: 'Пропущено', color: '#f87171', hint: 'Зрив без жодної тривоги' },
  { key: 'false_alarm', label: 'Хибна тривога', color: '#c084fc', hint: 'Тривога на розряді, який дійшов до кінця без зриву' },
  { key: 'quiet', label: 'Тихий', color: '#475569', hint: 'Ні зриву, ні тривоги — звичайний випадок' },
  { key: 'excluded', label: 'Виключено', color: '#64748b', hint: 'Спонтанний зрив або контакт зі стінкою: передбачати нічого' },
]
const COLOR = Object.fromEntries(VERDICTS.map((v) => [v.key, v.color])) as Record<Verdict, string>

const W = 520
const H = 22

/** Thin stacked bar of mechanism shares. */
function ShareBar({ shares, width = 90, height = 8 }: { shares: Shares | undefined; width?: number; height?: number }) {
  if (!shares) return <span className="inline-block" style={{ width }} />
  const widths = MECHANISMS.map((m) => ((shares[m] ?? 0) / 100) * width)
  const starts = widths.map((_, i) => widths.slice(0, i).reduce((a, b) => a + b, 0))
  return (
    <svg width={width} height={height} className="shrink-0 rounded-sm bg-gray-800">
      {MECHANISMS.map((m, i) => (
        <rect key={m} x={starts[i]} y={0} width={widths[i]} height={height} fill={MECH_INFO[m].color} />
      ))}
    </svg>
  )
}

function ShotRow({ shot, tMax, threshold }: { shot: ShotReport; tMax: number; threshold: number }) {
  const x = (t: number) => (t / tMax) * W
  const end = shot.t_end ?? shot.t_tq ?? tMax
  const poly = useMemo(() => {
    if (shot.staircase.length === 0) return ''
    const pts = [`${x(0)},${H}`]
    let prev = 0
    for (const [t, p] of shot.staircase) {
      pts.push(`${x(t)},${H - prev * H}`, `${x(t)},${H - p * H}`)
      prev = p
    }
    pts.push(`${x(end)},${H - prev * H}`, `${x(end)},${H}`)
    return pts.join(' ')
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shot, tMax])
  const c = COLOR[shot.verdict]
  const pred = shot.predicted_cause
  const predDom = pred ? (pred.dominant ?? dominantOf(pred.share_pct)) : null
  const trueDom = shot.true_cause ? (shot.true_cause.dominant ?? dominantOf(shot.true_cause.share_pct)) : null
  return (
    <div className="flex items-center gap-2 py-0.5">
      <span className="w-8 shrink-0 text-right font-mono text-[10px] text-gray-600 tabular-nums">{shot.shot_id}</span>
      <svg width={W} height={H} className="shrink-0">
        <rect x={0} y={0} width={x(end)} height={H} fill="#111827" />
        <polygon points={poly} fill={c} opacity={0.35} />
        <line x1={0} y1={H - threshold * H} x2={x(end)} y2={H - threshold * H} stroke="#6b7280" strokeWidth={0.5} strokeDasharray="2 2" />
        {shot.t_tq !== null && <line x1={x(shot.t_tq)} y1={0} x2={x(shot.t_tq)} y2={H} stroke="#ef4444" strokeWidth={1.5} />}
        {shot.t_alarm !== null && <line x1={x(shot.t_alarm)} y1={0} x2={x(shot.t_alarm)} y2={H} stroke={c} strokeWidth={1.5} />}
      </svg>
      <span className="w-16 shrink-0 font-mono text-[10px] tabular-nums" style={{ color: c }}>
        {shot.warning_s !== null && shot.verdict !== 'false_alarm' ? `+${shot.warning_s.toFixed(2)}с`
          : shot.verdict === 'false_alarm' && shot.t_alarm !== null ? `@${shot.t_alarm.toFixed(1)}с` : ''}
      </span>
      {pred && (
        <span className="flex items-center gap-1 shrink-0" title={`Модель (у піку своєї λ): ${MECHANISMS.map((m) => `${MECH_INFO[m].short} ${(pred.share_pct[m] ?? 0).toFixed(0)}%`).join(', ')}`}>
          <ShareBar shares={pred.share_pct} width={60} />
          <span className="w-16 text-[10px] truncate" style={{ color: predDom ? MECH_INFO[predDom].color : undefined }}>
            {predDom ? MECH_INFO[predDom].short : ''}
          </span>
        </span>
      )}
      {trueDom && shot.disrupted && (
        <span className="text-[10px] shrink-0 w-24 truncate" title="Справжня причина в момент жереба (рушій)">
          {predDom ? (predDom === trueDom ? '✓ ' : '✗ ') : 'істина: '}
          <span style={{ color: MECH_INFO[trueDom].color }}>{MECH_INFO[trueDom].short}</span>
        </span>
      )}
      <span className="text-[10px] text-gray-600 truncate">{shot.scenario}</span>
    </div>
  )
}

function Tile({ label, value, color, hint }: { label: string; value: string; color: string; hint: string }) {
  return (
    <div className="bg-gray-900 border border-gray-800 rounded p-2" title={hint}>
      <div className="text-[9px] text-gray-600 uppercase tracking-wider">{label}</div>
      <div className="font-mono text-lg tabular-nums" style={{ color }}>{value}</div>
    </div>
  )
}

function MechanismBlock({ m }: { m: MechanismSummary }) {
  const rows = MECHANISMS.map((k) => {
    const row = m.confusion[k] ?? {}
    const total = Object.values(row).reduce((a, b) => a + (b ?? 0), 0)
    return { k, total, right: row[k] ?? 0, row }
  })
  return (
    <div className="bg-gray-900 border border-gray-800 rounded p-3 space-y-2">
      <div className="flex items-baseline justify-between">
        <span className="text-[10px] text-gray-500 uppercase tracking-wider">Чинники зриву</span>
        <span className="text-[10px] text-gray-500">
          головний механізм вгадано в <span className="text-emerald-400 font-mono">{m.agree} / {m.n}</span> зривах ({pct(m.accuracy)})
        </span>
      </div>
      <p className="text-[9px] text-gray-600 leading-snug">
        «Насправді» — розклад λ рушія в момент жереба зриву. «Модель» — її розклад у момент, коли її власна λ була найбільшою
        (при малій λ частки — шум). Частка механізму — імовірність, що зрив, якби він стався зараз, спричинив саме він.
      </p>
      <div className="grid grid-cols-[1fr_auto_auto] gap-x-3 gap-y-1 items-center text-[10px]">
        <span className="text-gray-500">механізм</span>
        <span className="text-gray-500 text-right">середня частка: насправді / модель</span>
        <span className="text-gray-500 text-right">вгадано</span>
        {rows.map(({ k, total, right, row }) => (
          <div key={k} className="contents">
            <span className="flex items-center gap-1.5" title={MECH_INFO[k].hint}>
              <span className="w-2 h-2 rounded-sm" style={{ background: MECH_INFO[k].color }} />
              <span className="text-gray-300">{MECH_INFO[k].label}</span>
            </span>
            <span className="font-mono text-right text-gray-400 tabular-nums">
              {(m.mean_true_share_pct[k] ?? 0).toFixed(0)}% / {(m.mean_predicted_share_pct[k] ?? 0).toFixed(0)}%
            </span>
            <span className="font-mono text-right tabular-nums text-gray-400"
              title={total ? `модель назвала: ${Object.entries(row).map(([p, c]) => `${MECH_INFO[p as keyof typeof MECH_INFO]?.short ?? p} ${c}`).join(', ')}` : ''}>
              {total ? `${right} / ${total}` : '—'}
            </span>
          </div>
        ))}
      </div>
      <div className="flex items-center gap-2 text-[9px] text-gray-500">
        <span>середній розклад:</span>
        <span>насправді</span><ShareBar shares={m.mean_true_share_pct} width={120} />
        <span>модель</span><ShareBar shares={m.mean_predicted_share_pct} width={120} />
      </div>
    </div>
  )
}

export default function TestRunPanel({ onClose }: { onClose: () => void }) {
  const [models, setModels] = useState<LocalModel[]>([])
  const [modelsDir, setModelsDir] = useState('')
  const [choice, setChoice] = useState<string>('')
  const [uploaded, setUploaded] = useState<UploadedModel | null>(null)
  const [uploading, setUploading] = useState<string | null>(null)
  const [dragOver, setDragOver] = useState(false)
  const [lead, setLead] = useState(0.1)
  const [budget, setBudget] = useState(0.05)
  const [shots, setShots] = useState(200)
  const [seed, setSeed] = useState(1)
  const [running, setRunning] = useState(false)
  const [done, setDone] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const [report, setReport] = useState<RunReport | null>(null)
  const [fresh, setFresh] = useState(false)
  const [hidden, setHidden] = useState<Set<Verdict>>(new Set())
  const dirInput = useRef<HTMLInputElement>(null)

  useEffect(() => {
    const el = dirInput.current
    if (el) { el.setAttribute('webkitdirectory', ''); el.setAttribute('directory', '') }
    api.info().then((i) => { setShots(i.eval_shots); setSeed(i.eval_seed) }).catch(() => {})
    api.models().then((m) => {
      setModels(m.models)
      setModelsDir(m.dir)
      const ok = m.models.find((x) => x.problems.length === 0)
      if (ok) setChoice(ok.id)
    }).catch((e) => setError(`сервер fusionsim недоступний: ${e.message}`))
  }, [])

  const handleFiles = useCallback(async (files: { file: File; path: string }[]) => {
    if (files.length === 0) return
    setError(null); setUploading('0 / ?')
    try {
      const u = await api.upload(files, (n, t) => setUploading(`${n} / ${t}`))
      setUploaded(u)
      setChoice('uploaded')
    } catch (e) {
      setUploaded(null); setError(e instanceof Error ? e.message : String(e))
    } finally {
      setUploading(null)
    }
  }, [])

  const selected = choice === 'uploaded' ? null : models.find((m) => m.id === choice) ?? null
  const problems = choice === 'uploaded' ? uploaded?.problems ?? ['перетягніть теку здачі'] : selected?.problems ?? ['оберіть модель']
  const canRun = !running && problems.length === 0

  const run = useCallback(async () => {
    setRunning(true); setError(null); setReport(null); setDone(0); setFresh(false)
    try {
      await api.validate({ model: choice, lead_time_s: lead, fpr_budget: budget, shots, seed })
      for (;;) {
        await new Promise((r) => setTimeout(r, 400))
        const p = await api.validateProgress()
        setDone(p.done)
        if (p.error) throw new Error(p.error)
        if (!p.running) {
          if (!p.result) throw new Error('залік скінчився без звіту')
          setReport(p.result); setFresh(true)
          break
        }
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setRunning(false)
    }
  }, [choice, lead, budget, shots, seed])

  const rescore = useCallback(async () => {
    if (!report) return
    try {
      setReport(await api.rescore(report, lead, budget))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }, [report, lead, budget])

  const openReport = async (f: File) => {
    try {
      const j = JSON.parse(await f.text())
      if (!Array.isArray(j.shots)) throw new Error('у файлі немає розкладу по розрядах')
      setReport(j); setFresh(false); setError(null)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e)); setReport(null)
    }
  }

  const counts = useMemo(() => {
    const c = Object.fromEntries(VERDICTS.map((v) => [v.key, 0])) as Record<Verdict, number>
    for (const s of report?.shots ?? []) c[s.verdict] = (c[s.verdict] ?? 0) + 1
    return c
  }, [report])
  const visible = useMemo(() => (report?.shots ?? []).filter((s) => !hidden.has(s.verdict)), [report, hidden])
  const tMax = useMemo(() => Math.max(1, ...(report?.shots ?? []).map((s) => s.t_end ?? s.t_tq ?? 0)), [report])
  const S = report?.summary
  const thr = typeof S?.threshold === 'number' ? S.threshold : 0.5

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/70 p-4" onClick={running ? undefined : onClose}>
      <div className="w-full max-w-5xl max-h-full flex flex-col bg-[#0d1117] border border-gray-700 rounded-lg shadow-2xl shadow-black/60"
        onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between p-3 border-b border-gray-800">
          <div>
            <h2 className="panel-title">Залік моделі (Test run)</h2>
            <p className="text-[9px] text-gray-600 mt-0.5">
              Модель отримує 100 каналів на кожен крок 5 мс свіжих розрядів і повертає ймовірність зриву p; у кінці — звіт як у toksym
              і, якщо модель це вміє, розклад причин зриву
            </p>
          </div>
          <button onClick={onClose} className="text-gray-500 hover:text-gray-300 cursor-pointer text-lg">✕</button>
        </div>

        <div className="flex-1 overflow-y-auto p-3 space-y-3">
          <div className="bg-gray-900 border border-gray-800 rounded p-3 space-y-2">
            <label className="text-[10px] text-gray-500 uppercase tracking-wider">Модель</label>
            <select value={choice} disabled={running} onChange={(e) => setChoice(e.target.value)}
              className="w-full bg-black border border-gray-700 rounded px-2 py-1 text-xs text-cyan-400 font-mono focus:outline-none">
              <option value="" disabled>— оберіть здачу —</option>
              {models.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.folder}{m.name ? ` · ${m.name}` : ''}{m.problems.length ? '  ⚠' : ''}
                </option>
              ))}
              <option value="uploaded">{uploaded?.manifest?.name ? `Перетягнута тека · ${uploaded.manifest.name}` : 'Перетягнута тека…'}</option>
            </select>
            <p className="text-[9px] text-gray-600">Здачі читаються з <code className="text-gray-500">{modelsDir || '…'}</code> (ключ --models-dir); або перетягніть будь-яку теку нижче.</p>

            <div
              onClick={() => dirInput.current?.click()}
              onDragOver={(e) => { e.preventDefault(); setDragOver(true) }}
              onDragLeave={() => setDragOver(false)}
              onDrop={(e) => { e.preventDefault(); setDragOver(false); filesFromDrop(e.dataTransfer.items).then(handleFiles) }}
              className={`border border-dashed rounded p-3 text-center cursor-pointer transition-colors ${dragOver ? 'border-emerald-500 bg-emerald-950/20' : 'border-gray-700 hover:border-gray-500'}`}>
              {uploading ? <p className="text-sm text-gray-400 font-mono">завантаження {uploading}</p> : (
                <>
                  <p className="text-sm text-gray-300">Перетягніть сюди теку здачі або клацніть, щоб вибрати</p>
                  <p className="text-[10px] text-gray-600 mt-1">тека з <code className="text-gray-500">toksym.json</code>, скриптом моделі і навченими вагами</p>
                </>
              )}
            </div>
            <input ref={dirInput} type="file" multiple className="hidden"
              onChange={(e) => { handleFiles(filesFromInput(e.target.files ?? new DataTransfer().files)); e.target.value = '' }} />

            <div className="text-[10px] space-y-1">
              {choice === 'uploaded' && uploaded?.manifest && (
                <div className="font-mono text-gray-400">↳ {uploaded.root ? `${uploaded.root}/` : ''}{uploaded.manifest.entry}
                  <span className="text-gray-600"> · файлів: {uploaded.files.length}{uploaded.manifest.files?.length ? ` · ваги: ${uploaded.manifest.files.join(', ')}` : ''}</span>
                </div>
              )}
              {selected && <div className="font-mono text-gray-400">↳ {selected.entry ?? '?'} <span className="text-gray-600">· файлів: {selected.files}</span></div>}
              {(selected?.notes || (choice === 'uploaded' && uploaded?.manifest?.notes)) && (
                <div className="text-gray-500 italic">{selected?.notes ?? uploaded?.manifest?.notes}</div>
              )}
              {problems.length > 0 ? (
                <ul className="text-red-400 list-disc pl-4">{problems.map((p) => <li key={p}>{p}</li>)}</ul>
              ) : <div className="text-emerald-500">готово до запуску</div>}
              {choice === 'uploaded' && uploaded?.suggestion && (
                <>
                  <div className="text-gray-500">Збережіть як toksym.json поруч зі скриптом:</div>
                  <pre className="mt-1 bg-black border border-gray-800 rounded p-2 text-[10px] text-gray-300 overflow-x-auto">{uploaded.suggestion}</pre>
                </>
              )}
            </div>

            <div className="grid grid-cols-4 gap-2">
              {([
                ['Випередження (lead), с', 'Наскільки завчасно до термічного гасіння має спрацювати тривога', lead, setLead, 0.01, 0.005, 5],
                ['Бюджет хибних тривог', 'Яка частка розрядів без зриву може мати тривогу (0.05 = 5 %)', budget, setBudget, 0.01, 0, 1],
                ['Розрядів', 'Скільки розрядів прогнати (кожен ~3 с з моделлю)', shots, setShots, 1, 1, 2000],
                ['Seed', 'Той самий seed — ті самі розряди', seed, setSeed, 1, 0, 2 ** 31 - 1],
              ] as [string, string, number, (v: number) => void, number, number, number][]).map(([label, hint, v, set, step, min, max]) => (
                <label key={label} className="block" title={hint}>
                  <span className="text-[9px] text-gray-500">{label}</span>
                  <input type="number" value={v} step={step} min={min} max={max} disabled={running}
                    onChange={(e) => set(Number(e.target.value))}
                    className="mt-0.5 w-full bg-black border border-gray-700 rounded px-1.5 py-1 font-mono text-[11px] text-gray-200 tabular-nums disabled:opacity-50" />
                </label>
              ))}
            </div>
            <div className="flex items-center gap-3">
              <button onClick={run} disabled={!canRun}
                className="px-3 py-1 rounded text-[11px] font-medium bg-emerald-700 hover:bg-emerald-600 disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer text-white">
                {running ? 'Іде залік…' : '▶ Запустити модель'}
              </button>
              {running && (
                <>
                  <span className="text-[10px] font-mono text-gray-500">{done} / {shots} розрядів</span>
                  <button onClick={() => api.validateCancel()} className="text-[10px] text-amber-500 hover:text-amber-400 cursor-pointer">зупинити</button>
                </>
              )}
              {report && !running && (
                <button onClick={rescore} title="Перерахувати цей звіт з полями вище (lead, бюджет) без повторного прогону"
                  className="px-2 py-1 rounded text-[11px] bg-gray-800 hover:bg-gray-700 cursor-pointer text-gray-300">↻ Перерахувати з цими полями</button>
              )}
              <label className="ml-auto text-[10px] text-gray-500 hover:text-gray-300 cursor-pointer">
                відкрити збережений run.json
                <input type="file" accept=".json" className="hidden" onChange={(e) => { const f = e.target.files?.[0]; if (f) openReport(f); e.target.value = '' }} />
              </label>
            </div>
            {running && (
              <div className="h-1.5 bg-gray-800 rounded overflow-hidden">
                <div className="h-full bg-emerald-500 transition-[width] duration-200" style={{ width: `${shots ? (done / shots) * 100 : 0}%` }} />
              </div>
            )}
          </div>

          {error && <pre className="text-[11px] text-red-400 font-mono whitespace-pre-wrap">{error}</pre>}

          {report && S && (
            <>
              {fresh && (
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="text-[10px] text-gray-500 uppercase tracking-wider">Дані прогону</span>
                  {(['measured.csv', 'shots.csv', 'run.json'] as const).map((n) => (
                    <a key={n} href={api.validateFile(n)} download
                      className="px-2 py-0.5 rounded border border-gray-700 text-gray-300 hover:text-white hover:border-gray-500 font-mono text-[10px]">⤓ {n}</a>
                  ))}
                  <span className="text-[9px] text-gray-600">measured.csv — 100 каналів, p моделі і (якщо є) її λ та частки на кожному кроці; shots.csv — мітки, тривога, вердикт</span>
                </div>
              )}
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-2">
                <Tile label="Спіймано (Caught)" value={pct(S.tpr)} color="#34d399" hint="Частка зривів, на які тривога прийшла щонайменше за lead до гасіння" />
                <Tile label="Хибні тривоги" value={pct(S.fpr)} color="#c084fc" hint="Частка розрядів без зриву, на яких була тривога" />
                <Tile label="Поріг (Threshold)" value={typeof S.threshold === 'number' ? S.threshold.toFixed(3) : '—'} color="#94a3b8" hint="Підібраний поріг p: найбільше спійманих при хибних ≤ бюджету" />
                <Tile label="Медіанне випередження" value={typeof S.median_warning_s === 'number' ? `${S.median_warning_s.toFixed(2)} с` : '—'} color="#94a3b8" hint="Медіана часу від тривоги до гасіння серед спійманих" />
              </div>
              <div className="text-[10px] font-mono text-gray-500">
                {report.model && <>модель {report.model} · </>}
                спіймано {S.tp} з {S.n_disruptions_scored} · запізно {S.late_alarms} · хибних {S.fp} з {S.n_quiet} ·{' '}
                {S.n_shots} розрядів · {S.n_disrupted} зривів · {S.n_spontaneous} спонтанних ·
                lead {Math.round(report.rule.lead_time_s * 1e3)} мс · бюджет {pct(report.rule.fpr_budget)}
                {report.timing && <> · крок моделі p50 {report.timing.latency_p50_ms.toFixed(2)} мс, p99 {report.timing.latency_p99_ms.toFixed(2)} мс</>}
              </div>

              {S.mechanisms ? <MechanismBlock m={S.mechanisms} /> : (
                <p className="text-[10px] text-gray-600">
                  Чинники зриву: ця модель повертає лише p. Щоб бачити розклад причин, модель має віддавати частки механізмів
                  (див. models/impactnet або models/v8_impact і docs/USER_GUIDE.md).
                </p>
              )}

              <div className="flex flex-wrap gap-1.5">
                {VERDICTS.map((v) => {
                  const off = hidden.has(v.key)
                  return (
                    <button key={v.key} title={v.hint}
                      onClick={() => setHidden((h) => { const n = new Set(h); if (n.has(v.key)) n.delete(v.key); else n.add(v.key); return n })}
                      className={`flex items-center gap-1 px-2 py-0.5 rounded border text-[10px] cursor-pointer ${off ? 'border-gray-800 opacity-40' : 'border-gray-700'}`}>
                      <span className="w-2 h-2 rounded-sm" style={{ background: v.color }} />
                      <span className="text-gray-300">{v.label}</span>
                      <span className="text-gray-500 tabular-nums">{counts[v.key]}</span>
                    </button>
                  )
                })}
                <span className="flex items-center gap-2 ml-2 text-[9px] text-gray-500">
                  {MECHANISMS.map((m) => (
                    <span key={m} className="flex items-center gap-1" title={MECH_INFO[m].hint}>
                      <span className="w-2 h-2 rounded-sm" style={{ background: MECH_INFO[m].color }} />{MECH_INFO[m].short}
                    </span>
                  ))}
                </span>
              </div>
              <div className="overflow-x-auto">
                <div className="flex items-center gap-2 text-[9px] text-gray-600 pb-1">
                  <span className="w-8 text-right">№</span>
                  <span style={{ width: W }}>0 … {tMax.toFixed(0)} с · бігучий максимум p, поріг пунктиром, червоне — гасіння</span>
                  <span className="w-16">випередж.</span>
                  {S.mechanisms && <span className="w-[136px]">причина за моделлю</span>}
                  {S.mechanisms && <span className="w-24">насправді</span>}
                  <span>сценарій</span>
                </div>
                {visible.map((s) => <ShotRow key={s.shot_id} shot={s} tMax={tMax} threshold={thr} />)}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  )
}
