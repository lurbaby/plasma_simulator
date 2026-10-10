import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type LiveInfo, type LocalModel } from '../lib/api'
import { MECHANISMS, MECH_INFO, dominantOf, type Mechanism, type Shares } from '../lib/mechanisms'
import type { Snapshot } from '../lib/types'
import type { OverrideChannel, SimControls } from '../lib/useSimulation'

// Actuators: (channel, label, unit, min, max, step, programmed value, what it does)
const ACTUATORS: [OverrideChannel, string, string, number, number, number, (s: Snapshot) => number, string][] = [
  ['ip', 'Iₚ — струм плазми', 'МА', 0.3, 5, 0.01, (s) => s.prog_ip, 'Більший Iₚ: вища межа Грінвальда, але нижчий q95 (кінк)'],
  ['bt', 'Bₜ — тороїдальне поле', 'Т', 1.5, 3.45, 0.01, (s) => s.prog_bt, 'Більше Bₜ: вищий q95 (далі від кінку), нижчий βN'],
  ['ne_target', 'n̄ₑ — уставка густини', '10²⁰м⁻³', 0, 1.6, 0.01, (s) => s.prog_ne, 'Густина тягнеться до уставки з τ ≈ 0.3 с; f_GW = n̄ₑ / n_GW'],
  ['d2_puff', 'D₂ — напуск дейтерію', '10²⁰/с', 0, 8, 0.05, (s) => s.d2_puff, 'Додаткове джерело густини понад уставку'],
  ['neon_puff', 'Ne — засів неону', '10²⁰/с', 0, 2.5, 0.01, (s) => s.neon_puff, 'Домішка накопичується з τ ≈ 0.5 с і випромінює: росте P_rad'],
  ['p_nbi', 'NBI — нагрів пучком', 'МВт', 0, 34, 0.5, (s) => s.prog_p_nbi, 'Більше нагріву: нижча P_rad/P_in, але вищий βN'],
  ['p_ich', 'ICRH — ВЧ-нагрів', 'МВт', 0, 9, 0.1, (s) => s.prog_p_ich, 'Те саме, що NBI'],
  ['p_ech', 'ECH — ЕЦ-нагрів', 'МВт', 0, 6, 0.1, (s) => s.prog_p_ech, 'Те саме, що NBI'],
  ['kappa', 'κ — витягнутість', '', 1.0, 2.0, 0.01, (s) => s.prog_kappa ?? 1.7, 'Більша κ: вищий q95 і краще утримання'],
  ['delta', 'δ — трикутність', '', 0, 0.6, 0.01, (s) => s.prog_delta ?? 0.25, 'Більша δ: трохи вищий q95; δ < 0.1 — обмежена (limited) плазма'],
]

type Source = 'model' | 'engine'
interface Action { t: number; text: string }
interface Advice { mech: Mechanism; basis: string; set: [OverrideChannel, number][]; text: string }

/** λ split from the engine's own risk factors (the "oracle"), as ImpactNet defines shares. */
function engineShares(s: Snapshot): Shares {
  const r: Record<Mechanism, number> = {
    greenwald: (s.risk_greenwald ?? 0) ** 3,
    q95: (s.risk_q95 ?? 0) ** 3,
    radiation: (s.risk_radiation ?? 0) ** 3,
    troyon: (s.risk_beta ?? 0) ** 3,
    other: s.risk_locked_mode ?? 0,
  }
  const c = MECHANISMS.map((m) => -Math.log(1 - Math.min(r[m], 1 - 1e-9)))
  const sum = c.reduce((a, b) => a + b, 0) || 1
  return Object.fromEntries(MECHANISMS.map((m, i) => [m, (100 * c[i]) / sum])) as Shares
}

/** Nearest limit from the plasma state, when no meaningful shares are available. */
function nearestLimit(s: Snapshot): Mechanism {
  const v: [Mechanism, number][] = [
    ['greenwald', s.f_greenwald / 1.0],
    ['q95', 2.2 / Math.max(s.q95, 0.5) / 0.9],
    ['radiation', s.p_input > 1 ? s.p_rad / s.p_input / 0.8 : 0],
    ['troyon', s.beta_n / 2.8 / 0.85],
  ]
  return v.reduce((a, b) => (b[1] > a[1] ? b : a))[0]
}

/** The control policy: what to change against each mechanism. Edit here to plug in your own. */
function advise(mech: Mechanism, s: Snapshot, basis: string): Advice | null {
  switch (mech) {
    case 'greenwald': {
      const ne = Math.max(0.2, Math.min(s.prog_ne, s.ne_bar) * 0.85)
      return { mech, basis, set: [['ne_target', ne], ['d2_puff', Math.max(0, s.d2_puff * 0.5)]],
        text: `знизити уставку густини до ${ne.toFixed(2)} і напуск D₂ удвічі (f_GW = ${s.f_greenwald.toFixed(2)})` }
    }
    case 'q95': {
      const ip = Math.max(1.0, s.prog_ip * Math.min(0.95, s.q95 / 3.0))
      return { mech, basis, set: [['ip', ip]], text: `знизити Iₚ до ${ip.toFixed(2)} МА, щоб q95 → 3 (зараз ${s.q95.toFixed(2)})` }
    }
    case 'radiation': {
      const nbi = Math.min(34, s.prog_p_nbi + 5)
      return { mech, basis, set: [['neon_puff', 0], ['p_nbi', nbi]],
        text: `вимкнути неон і додати NBI до ${nbi.toFixed(1)} МВт (P_rad/P_in = ${(s.p_input > 1 ? s.p_rad / s.p_input : 0).toFixed(2)})` }
    }
    case 'troyon': {
      const nbi = Math.max(0, s.prog_p_nbi * 0.8)
      return { mech, basis, set: [['p_nbi', nbi]], text: `зменшити NBI до ${nbi.toFixed(1)} МВт (βN = ${s.beta_n.toFixed(2)})` }
    }
    default:
      return null
  }
}

function ShareRows({ shares, lam }: { shares: Shares | null; lam: number | null }) {
  if (!shares) return <p className="text-[9px] text-gray-600">розкладу причин немає</p>
  const dom = dominantOf(shares)
  return (
    <div className="space-y-0.5">
      {MECHANISMS.map((m) => {
        const v = shares[m] ?? 0
        return (
          <div key={m} className="flex items-center gap-1.5 text-[10px]" title={MECH_INFO[m].hint}>
            <span className={`w-20 truncate ${m === dom ? 'text-gray-100' : 'text-gray-400'}`}>{MECH_INFO[m].short}</span>
            <div className="flex-1 h-1.5 bg-gray-800 rounded overflow-hidden">
              <div className="h-full" style={{ width: `${v}%`, background: MECH_INFO[m].color }} />
            </div>
            <span className="w-10 text-right font-mono tabular-nums text-gray-400">{v.toFixed(0)}%</span>
          </div>
        )
      })}
      {lam !== null && lam < 0.02 && (
        <p className="text-[9px] text-gray-600">λ = {lam.toFixed(4)} 1/с — ризику майже немає, частки зараз нічого не означають</p>
      )}
    </div>
  )
}

export default function ControlPanel({ snapshot, controls, running, onClose }: {
  snapshot: Snapshot | null
  controls: SimControls
  running: boolean
  onClose: () => void
}) {
  const [values, setValues] = useState<Partial<Record<OverrideChannel, number>>>({})
  const [models, setModels] = useState<LocalModel[]>([])
  const [model, setModel] = useState('')
  const [source, setSource] = useState<Source>('engine')
  const [connected, setConnected] = useState<string | null>(null)
  const [p, setP] = useState(0)
  const [lam, setLam] = useState<number | null>(null)
  const [shares, setShares] = useState<Shares | null>(null)
  const [pHist, setPHist] = useState<number[]>([])
  const [advice, setAdvice] = useState<Advice | null>(null)
  const [auto, setAuto] = useState(false)
  const [threshold, setThreshold] = useState(0.3)
  const [hold, setHold] = useState(0.3)
  const [minLam, setMinLam] = useState(0.02)
  const [log, setLog] = useState<Action[]>([])
  const [error, setError] = useState<string | null>(null)

  const queue = useRef<{ t: number; v: number[] }[]>([])
  const inFlight = useRef(false)
  const lastAction = useRef(-Infinity)
  const lastSnap = useRef<Snapshot | null>(null)
  const cfg = useRef({ auto, threshold, hold, minLam, source, connected })
  useEffect(() => { cfg.current = { auto, threshold, hold, minLam, source, connected } },
    [auto, threshold, hold, minLam, source, connected])

  useEffect(() => {
    api.models().then((m) => {
      setModels(m.models)
      const ok = m.models.find((x) => x.problems.length === 0 && /impact/i.test(x.folder ?? ''))
        ?? m.models.find((x) => x.problems.length === 0)
      if (ok) setModel(ok.id)
    }).catch(() => setError('сервер fusionsim недоступний — працює лише ручне керування і λ рушія'))
  }, [])

  const set = useCallback((ch: OverrideChannel, v: number | null) => {
    controls.setOverride(ch, v)
    setValues((x) => {
      const n = { ...x }
      if (v === null) delete n[ch]
      else n[ch] = v
      return n
    })
  }, [controls])

  const note = useCallback((t: number, text: string) => setLog((l) => [{ t, text }, ...l].slice(0, 60)), [])

  const apply = useCallback((a: Advice, t: number, prob: number | null) => {
    for (const [ch, v] of a.set) set(ch, v)
    note(t, `${prob !== null ? `p=${prob.toFixed(2)} ` : ''}${MECH_INFO[a.mech].short} (${a.basis}) → ${a.text}`)
  }, [set, note])

  /** One update: p, λ and shares from the predictor → advice → maybe act. */
  const update = useCallback((prob: number, l: number | null, sh: Shares | null, s: Snapshot) => {
    setP(prob)
    setLam(l)
    setShares(sh)
    const c = cfg.current
    // Shares are only meaningful once λ is noticeable; below that, read the limits off the plasma.
    const useShares = sh && (l === null || l >= c.minLam)
    const mech = useShares ? dominantOf(sh)! : nearestLimit(s)
    const a = advise(mech, s, useShares ? (c.source === 'model' ? 'за моделлю' : 'за рушієм') : 'найближча межа')
    setAdvice(a)
    if (!a || !c.auto || prob <= c.threshold || s.time - lastAction.current < c.hold) return
    lastAction.current = s.time
    apply(a, s.time, prob)
  }, [apply])

  const flush = useCallback(async () => {
    if (inFlight.current || queue.current.length === 0 || !cfg.current.connected) return
    inFlight.current = true
    const batch = queue.current.splice(0, queue.current.length)
    try {
      const r = await api.liveStep(batch)
      const info = (r.info ?? []).filter(Boolean) as LiveInfo[]
      const last = info[info.length - 1]
      // For decisions use the instantaneous probability if the model sends one (p_now),
      // not a running maximum that never falls after an action.
      const pNow = last?.p_now ?? r.p[r.p.length - 1] ?? 0
      setPHist((h) => [...h, ...r.p.filter((_, i) => i % 3 === 0)].slice(-400))
      if (lastSnap.current) update(pNow, last?.risk_total ?? null, last?.share_pct ?? null, lastSnap.current)
    } catch (e) {
      setConnected(null)
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      inFlight.current = false
      if (queue.current.length) flush()
    }
  }, [update])

  // Every physics step: feed the model (or read the engine's λ), then advise / act.
  useEffect(() => controls.subscribeSteps((snaps) => {
    const s = snaps[snaps.length - 1]
    if (!s) return
    lastSnap.current = s
    if (cfg.current.source === 'engine') {
      const prob = 1 - Math.exp(-s.disruption_risk * 1.0)
      setPHist((h) => [...h, prob].slice(-400))
      update(prob, s.disruption_risk, engineShares(s), s)
    } else if (cfg.current.connected) {
      for (const x of snaps) if (x.channels) queue.current.push({ t: x.time, v: x.channels })
      flush()
    }
  }), [controls, flush, update])

  const connect = async () => {
    setError(null)
    try {
      const r = await api.liveStart(model, 20)
      setConnected(r.model ?? model)
      setPHist([])
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }
  const disconnect = async () => {
    setConnected(null)
    await api.liveStop().catch(() => {})
  }
  useEffect(() => () => { api.liveStop().catch(() => {}) }, [])

  const releaseAll = () => {
    controls.clearOverrides()
    setValues({})
    note(snapshot?.time ?? 0, 'усі зміни скинуто — розряд знову йде за програмою')
  }

  const spark = pHist.length > 1
    ? pHist.map((v, i) => `${(i / (pHist.length - 1)) * 260},${40 - v * 40}`).join(' ') : ''

  return (
    <div className="fixed right-2 top-14 bottom-2 z-[55] w-[360px] flex flex-col bg-[#0d1117]/95 border border-gray-700 rounded-lg shadow-2xl shadow-black/60">
      <div className="flex items-center justify-between p-2 border-b border-gray-800">
        <div>
          <h2 className="panel-title">Керування розрядом</h2>
          <p className="text-[9px] text-gray-600">Змінюйте уставки просто під час розряду — вручну або за прогнозом моделі</p>
        </div>
        <button onClick={onClose} className="text-gray-500 hover:text-gray-300 cursor-pointer text-lg">✕</button>
      </div>
      <div className="flex-1 overflow-y-auto p-2 space-y-3">
        <section className="space-y-1.5">
          <span className="text-[10px] text-gray-500 uppercase tracking-wider">1. Прогноз зриву</span>
          <div className="flex rounded overflow-hidden border border-gray-700">
            {([['engine', 'Рушій (оракул λ)'], ['model', 'Ваша модель']] as [Source, string][]).map(([k, l]) => (
              <button key={k} onClick={() => setSource(k)}
                title={k === 'engine' ? 'Справжня λ симулятора — ідеальний прогноз для перевірки політики керування' : 'Модель з --models-dir отримує ті самі 100 каналів, що на заліку'}
                className={`flex-1 px-2 py-1 text-[11px] cursor-pointer ${source === k ? 'bg-cyan-600 text-white' : 'bg-gray-800 text-gray-400 hover:bg-gray-700'}`}>{l}</button>
            ))}
          </div>
          {source === 'model' && (
            <div className="space-y-1">
              <select value={model} onChange={(e) => setModel(e.target.value)} disabled={!!connected}
                className="w-full bg-black border border-gray-700 rounded px-2 py-1 text-[11px] text-cyan-400 font-mono">
                {models.map((m) => <option key={m.id} value={m.id}>{m.folder}{m.name ? ` · ${m.name}` : ''}{m.problems.length ? ' ⚠' : ''}</option>)}
                <option value="uploaded">Тека, перетягнута в «Залік»</option>
              </select>
              {connected ? (
                <button onClick={disconnect} className="w-full px-2 py-1 rounded text-[11px] bg-gray-700 hover:bg-gray-600 cursor-pointer">■ Від'єднати {connected}</button>
              ) : (
                <button onClick={connect} disabled={!model}
                  className="w-full px-2 py-1 rounded text-[11px] bg-emerald-700 hover:bg-emerald-600 disabled:opacity-40 cursor-pointer">▶ Під'єднати модель (до натискання Start)</button>
              )}
              <p className="text-[9px] text-gray-600">Причини зриву показують моделі, що віддають частки механізмів (impactnet, v8_impact).</p>
            </div>
          )}
          <div className="flex items-center gap-2">
            <div title="Імовірність зриву: для рушія — за 1 с при поточній λ; для моделі — її миттєве p">
              <div className="font-mono text-2xl tabular-nums leading-none" style={{ color: p > threshold ? '#f87171' : '#34d399' }}>{p.toFixed(3)}</div>
              <div className="text-[9px] text-gray-500 font-mono">λ {lam !== null ? lam.toFixed(3) : '—'} 1/с</div>
            </div>
            <svg width={260} height={40} className="bg-gray-900 rounded">
              <line x1={0} x2={260} y1={40 - threshold * 40} y2={40 - threshold * 40} stroke="#6b7280" strokeDasharray="2 2" strokeWidth={0.5} />
              {spark && <polyline points={spark} fill="none" stroke="#38bdf8" strokeWidth={1} />}
            </svg>
          </div>
          <div className="text-[10px] text-gray-500 uppercase tracking-wider pt-1">Чинники зриву зараз</div>
          <ShareRows shares={shares} lam={lam} />
        </section>

        <section className="space-y-1.5 border-t border-gray-800 pt-2">
          <span className="text-[10px] text-gray-500 uppercase tracking-wider">2. Порадник і автокерування</span>
          <div className="bg-gray-900 rounded p-2 text-[10px] space-y-1">
            {advice ? (
              <>
                <div><span className="text-gray-500">Головна загроза:</span>{' '}
                  <span style={{ color: MECH_INFO[advice.mech].color }}>{MECH_INFO[advice.mech].label}</span>
                  <span className="text-gray-600"> ({advice.basis})</span></div>
                {p <= threshold ? (
                  <div className="text-gray-500">p = {p.toFixed(3)} нижче порогу {threshold} — зараз дія не потрібна. Якщо ризик зросте: {advice.text}</div>
                ) : (
                  <div className="text-gray-200">Рекомендація: {advice.text}</div>
                )}
                <button onClick={() => snapshot && apply(advice, snapshot.time, null)}
                  className="px-2 py-0.5 rounded bg-amber-700 hover:bg-amber-600 cursor-pointer text-white">Застосувати зараз</button>
              </>
            ) : <span className="text-gray-500">Запустіть розряд (Start) — тут з'явиться рекомендація.</span>}
          </div>
          <label className="flex items-center gap-2 cursor-pointer">
            <input type="checkbox" checked={auto} onChange={(e) => setAuto(e.target.checked)} className="accent-amber-500" />
            <span className="text-[11px] text-gray-200">Автокерування: застосовувати рекомендацію, коли p &gt; порогу</span>
          </label>
          <div className="grid grid-cols-3 gap-2">
            <label className="block" title="Коли p перевищує цей поріг, автокерування діє"><span className="text-[9px] text-gray-500">Поріг p</span>
              <input type="number" min={0} max={1} step={0.05} value={threshold} onChange={(e) => setThreshold(Number(e.target.value))}
                className="w-full bg-black border border-gray-700 rounded px-1.5 py-0.5 font-mono text-[11px]" /></label>
            <label className="block" title="Не частіше ніж раз на стільки секунд — плазмі треба час відреагувати"><span className="text-[9px] text-gray-500">Пауза між діями, с</span>
              <input type="number" min={0.05} max={5} step={0.05} value={hold} onChange={(e) => setHold(Number(e.target.value))}
                className="w-full bg-black border border-gray-700 rounded px-1.5 py-0.5 font-mono text-[11px]" /></label>
            <label className="block" title="Нижче цієї λ частки механізмів — шум, тож причину визначаємо за найближчою межею"><span className="text-[9px] text-gray-500">Мін. λ для часток</span>
              <input type="number" min={0} max={1} step={0.01} value={minLam} onChange={(e) => setMinLam(Number(e.target.value))}
                className="w-full bg-black border border-gray-700 rounded px-1.5 py-0.5 font-mono text-[11px]" /></label>
          </div>
          <table className="w-full text-[9px] text-gray-500">
            <tbody>
              <tr><td className="pr-2" style={{ color: MECH_INFO.greenwald.color }}>Грінвальд</td><td>n̄ₑ-уставка ×0.85, D₂ ×0.5</td></tr>
              <tr><td className="pr-2" style={{ color: MECH_INFO.q95.color }}>q95 (кінк)</td><td>Iₚ ↓ так, щоб q95 → 3</td></tr>
              <tr><td className="pr-2" style={{ color: MECH_INFO.radiation.color }}>Радіація</td><td>неон → 0, NBI +5 МВт</td></tr>
              <tr><td className="pr-2" style={{ color: MECH_INFO.troyon.color }}>Тройон</td><td>NBI ×0.8</td></tr>
              <tr><td className="pr-2" style={{ color: MECH_INFO.other.color }}>Інші</td><td>дії немає (передвісник або фон)</td></tr>
            </tbody>
          </table>
          {!running && <p className="text-[9px] text-gray-600">Натисніть Start угорі, щоб пустити розряд.</p>}
          <div className="max-h-36 overflow-y-auto space-y-0.5">
            {log.map((a, i) => (
              <div key={i} className="text-[10px] font-mono text-gray-400"><span className="text-gray-600">{a.t.toFixed(2)}с</span> {a.text}</div>
            ))}
          </div>
        </section>

        <section className="space-y-1.5 border-t border-gray-800 pt-2">
          <div className="flex items-center justify-between">
            <span className="text-[10px] text-gray-500 uppercase tracking-wider">3. Уставки вручну</span>
            <button onClick={releaseAll} className="text-[10px] text-amber-500 hover:text-amber-400 cursor-pointer">↺ повернути програму</button>
          </div>
          {ACTUATORS.map(([ch, label, unit, min, max, step, prog, hint]) => {
            const programmed = snapshot ? prog(snapshot) : NaN
            const ov = values[ch]
            const shown = ov ?? programmed
            return (
              <div key={ch} title={hint}>
                <div className="flex items-baseline justify-between text-[10px]">
                  <span className={ov !== undefined ? 'text-amber-400' : 'text-gray-300'}>{label}</span>
                  <span className="font-mono tabular-nums text-gray-400">
                    {Number.isFinite(shown) ? shown.toFixed(step < 0.1 ? 2 : 1) : '—'} {unit}
                    {ov !== undefined && (
                      <button onClick={() => set(ch, null)} title="Повернути запрограмовану хвилю"
                        className="ml-1 text-gray-500 hover:text-gray-200 cursor-pointer">✕</button>
                    )}
                  </span>
                </div>
                <input type="range" min={min} max={max} step={step}
                  value={Number.isFinite(shown) ? shown : min}
                  onChange={(e) => set(ch, Number(e.target.value))}
                  className={`w-full h-1 ${ov !== undefined ? 'accent-amber-500' : 'accent-cyan-600'}`} />
              </div>
            )
          })}
          {snapshot && (
            <div className="grid grid-cols-4 gap-1 text-[9px] font-mono text-gray-500 pt-1">
              <span title="Частка Грінвальда, межа 1.0">f_GW {snapshot.f_greenwald.toFixed(2)}</span>
              <span title="Запас стійкості, кінк ≈ 2.4">q95 {snapshot.q95.toFixed(2)}</span>
              <span title="Частка випромінювання, межа 0.8">P_rad/P_in {(snapshot.p_input > 1 ? snapshot.p_rad / snapshot.p_input : 0).toFixed(2)}</span>
              <span title="Нормована бета, межа Тройона ≈ 2.4">βN {snapshot.beta_n.toFixed(2)}</span>
            </div>
          )}
          <p className="text-[9px] text-gray-600">Помаранчевий повзунок — уставка змінена вами; ✕ повертає програму. Межі повзунків — фізично розумні діапазони.</p>
        </section>
        {error && <pre className="text-[10px] text-red-400 whitespace-pre-wrap">{error}</pre>}
      </div>
    </div>
  )
}
