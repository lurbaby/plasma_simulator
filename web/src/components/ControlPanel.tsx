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

// Fixer actions (crates/tok-sym-core/src/fixer.rs ACTIONS)
const ACTION_UA: Record<string, string> = {
  none: 'нічого не змінювати',
  'ne_-10': 'густина й D₂ −10 %',
  'ne_-20': 'густина й D₂ −20 %',
  neon_off: 'вимкнути неон',
  'ip_-10': 'струм Iₚ −10 %',
  'heat_+20': 'нагрів +20 %',
  'heat_-20': 'нагрів −20 %',
  relax: 'повернути уставки ближче до програми',
}

type Phase = 'normal' | 'alert' | 'applied' | 'stable' | 'disrupted'
interface FixEvent { t: number; kind: Phase; text: string }
interface Action { t: number; text: string }
interface ShadowStatus {
  t0: number; time: number; t_trigger: number | null; cause: Mechanism | null; lambda_at_trigger: number
  p_shadow: number; p_real: number; real_t_trigger: number | null; finished: boolean
}
type Popup = { kind: 'prevented' | 'summary'; shadow: ShadowStatus | null; realDisrupted: boolean; realTrigger: number | null } | null

const PHASE_UI: Record<Phase, { label: string; color: string; bg: string }> = {
  normal: { label: 'НОРМА', color: '#34d399', bg: 'rgba(6,78,59,0.85)' },
  alert: { label: '⚠ ПРОГНОЗ ЗРИВУ', color: '#fbbf24', bg: 'rgba(120,53,15,0.9)' },
  applied: { label: '⚡ FIXER ВНІС ЗМІНИ', color: '#22d3ee', bg: 'rgba(22,78,99,0.9)' },
  stable: { label: '✓ ПЛАЗМУ СТАБІЛІЗОВАНО', color: '#34d399', bg: 'rgba(6,78,59,0.9)' },
  disrupted: { label: '✖ ЗРИВ', color: '#f87171', bg: 'rgba(127,29,29,0.95)' },
}

const DEFAULT_MODEL = /v8_fixer/i

function ShareRows({ shares, lam }: { shares: Shares | null; lam: number | null }) {
  if (!shares) return <p className="text-[9px] text-gray-600">під'єднайте модель, щоб бачити причини</p>
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

function NoInterventionView({ popup, onClose }: { popup: NonNullable<Popup>; onClose: () => void }) {
  const color = popup.realDisrupted ? '#f87171' : '#34d399'
  return (
    <div className="fixed inset-0 z-[70] flex items-center justify-center bg-black/50 p-4" onClick={onClose}>
      <div className="w-full max-w-md bg-[#0d1117] border-2 rounded-lg shadow-2xl p-4 space-y-2 text-[12px] text-gray-200" style={{ borderColor: color }}
        onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-bold" style={{ color }}>{popup.realDisrupted ? '✖ Розряд закінчився зривом' : '✓ Розряд завершено без зриву'}</h2>
          <button onClick={onClose} className="text-gray-500 hover:text-gray-300 cursor-pointer text-lg">✕</button>
        </div>
        <p>{popup.realDisrupted
          ? `Зрив почався на t = ${popup.realTrigger?.toFixed(2)} с. Fixer не втручався: модель не подала сигналу вчасно або не була під'єднана.`
          : 'Fixer не втручався — ризик не перевищував порогу, плазма дійшла до кінця програми.'}</p>
      </div>
    </div>
  )
}

function PopupView({ popup, actions, onClose }: { popup: NonNullable<Popup>; actions: Action[]; onClose: () => void }) {
  if (!popup.shadow) return <NoInterventionView popup={popup} onClose={onClose} />
  const sh = popup.shadow
  const cause = sh.cause ? MECH_INFO[sh.cause]?.label ?? sh.cause : null
  const pct = (x: number) => `${(x * 100).toFixed(x < 0.1 ? 1 : 0)} %`
  let title: string
  let color: string
  let body: React.ReactNode
  if (popup.realDisrupted) {
    title = '✖ Fixer не встиг'
    color = '#f87171'
    body = <p>Справжня плазма зірвалася на t = {popup.realTrigger?.toFixed(2)} с.
      {sh.t_trigger !== null ? ` Без втручання зрив стався б на t = ${sh.t_trigger.toFixed(2)} с.` : ''}</p>
  } else if (sh.t_trigger !== null) {
    title = popup.kind === 'prevented' ? '✓ Зрив відвернуто' : '✓ Розряд завершено без зриву'
    color = '#34d399'
    body = (
      <>
        <p>Без втручання fixer ця плазма <b className="text-red-300">зірвалася б на t = {sh.t_trigger.toFixed(2)} с</b>
          {cause && <> — причина: <span style={{ color: MECH_INFO[sh.cause!]?.color }}>{cause}</span></>}
          {sh.lambda_at_trigger > 0 && <> (λ = {sh.lambda_at_trigger.toFixed(2)} 1/с)</>}.</p>
        <p className="text-gray-300">{popup.kind === 'prevented' ? 'Справжня плазма працює далі.' : 'Справжня плазма дійшла до кінця програми.'}</p>
      </>
    )
  } else {
    title = 'Розряд завершено'
    color = '#94a3b8'
    body = <p>Без втручання цього разу зриву не сталося б (випадковість), але ризик був вищим:</p>
  }
  return (
    <div className="fixed inset-0 z-[70] flex items-center justify-center bg-black/50 p-4" onClick={onClose}>
      <div className="w-full max-w-lg bg-[#0d1117] border-2 rounded-lg shadow-2xl p-4 space-y-3 text-[12px] text-gray-200" style={{ borderColor: color }}
        onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-bold" style={{ color }}>{title}</h2>
          <button onClick={onClose} className="text-gray-500 hover:text-gray-300 cursor-pointer text-lg">✕</button>
        </div>
        {body}
        <div className="grid grid-cols-2 gap-2">
          <div className="bg-red-950/40 border border-red-900 rounded p-2">
            <div className="text-[10px] text-gray-400 uppercase">без fixer</div>
            <div className="font-mono text-xl text-red-300">{sh.t_trigger !== null ? '✖ зрив' : pct(sh.p_shadow)}</div>
            <div className="text-[10px] text-gray-500">{sh.t_trigger !== null ? `на t = ${sh.t_trigger.toFixed(2)} с` : `імовірність зриву з t = ${sh.t0.toFixed(1)} с`}</div>
          </div>
          <div className="bg-emerald-950/40 border border-emerald-900 rounded p-2">
            <div className="text-[10px] text-gray-400 uppercase">з fixer</div>
            <div className="font-mono text-xl text-emerald-300">{popup.realDisrupted ? '✖ зрив' : pct(sh.p_real)}</div>
            <div className="text-[10px] text-gray-500">{popup.realDisrupted ? `на t = ${popup.realTrigger?.toFixed(2)} с` : `імовірність зриву з t = ${sh.t0.toFixed(1)} с`}</div>
          </div>
        </div>
        {actions.length > 0 && (
          <div>
            <div className="text-[10px] text-gray-500 uppercase">що зробив fixer (з t = {sh.t0.toFixed(2)} с)</div>
            <ul className="text-[11px] font-mono text-gray-300 max-h-32 overflow-y-auto">
              {[...actions].reverse().slice(0, 8).map((a, i) => <li key={i}><span className="text-gray-500">{a.t.toFixed(2)} с</span> {a.text}</li>)}
            </ul>
          </div>
        )}
        <p className="text-[10px] text-gray-500 leading-snug">
          Порівняння чесне: у момент першого втручання симуляцію скопійовано разом з генератором випадкових подій, і копія
          («тінь») пройшла той самий розряд без змін уставок. Обидві плазми бачили ті самі випадковості — різниця дає лише втручання fixer.
        </p>
      </div>
    </div>
  )
}

export default function ControlPanel({ snapshot, controls, running, open, onClose }: {
  snapshot: Snapshot | null
  controls: SimControls
  running: boolean
  /** The side panel is shown; the predictor, fixer, banner and pop-ups work regardless. */
  open: boolean
  onClose: () => void
}) {
  const [values, setValues] = useState<Partial<Record<OverrideChannel, number>>>({})
  const [models, setModels] = useState<LocalModel[]>([])
  const [model, setModel] = useState('')
  const [connected, setConnected] = useState<string | null>(null)
  const [connecting, setConnecting] = useState(false)
  const [p, setP] = useState(0)
  const [lam, setLam] = useState<number | null>(null)
  const [shares, setShares] = useState<Shares | null>(null)
  const [pHist, setPHist] = useState<number[]>([])
  const [fixerOn, setFixerOn] = useState(true)
  const [threshold, setThreshold] = useState(0.2)
  const [period, setPeriod] = useState(0.25)
  const [armSec, setArmSec] = useState(2.0)
  const [phase, setPhase] = useState<Phase>('normal')
  const [phaseText, setPhaseText] = useState('')
  const [events, setEvents] = useState<FixEvent[]>([])
  const [costs, setCosts] = useState<Record<string, number> | null>(null)
  const [lastAction, setLastAction] = useState<string | null>(null)
  const [noAction, setNoAction] = useState(false)
  const [log, setLog] = useState<Action[]>([])
  const [shadow, setShadow] = useState<ShadowStatus | null>(null)
  const [popup, setPopup] = useState<Popup>(null)
  const [error, setError] = useState<string | null>(null)

  const queue = useRef<{ t: number; v: number[]; extra?: unknown }[]>([])
  const inFlight = useRef(false)
  const newShot = useRef<Promise<unknown> | null>(null)
  const lastSnap = useRef<Snapshot | null>(null)
  const armedUntil = useRef(-Infinity)
  const lastDecisionT = useRef(-Infinity)
  const appliedAt = useRef<number | null>(null)
  const lastTime = useRef(0)
  const phaseRef = useRef<Phase>('normal')
  const popped = useRef({ prevented: false, summary: false })
  const [actions, setActions] = useState<Action[]>([])
  const cfg = useRef({ fixerOn, threshold, period, armSec, connected })
  useEffect(() => { cfg.current = { fixerOn, threshold, period, armSec, connected } },
    [fixerOn, threshold, period, armSec, connected])

  useEffect(() => {
    api.models().then((m) => {
      setModels(m.models)
      const ok = m.models.find((x) => x.problems.length === 0 && DEFAULT_MODEL.test(x.folder ?? ''))
        ?? m.models.find((x) => x.problems.length === 0)
      if (ok) setModel(ok.id)
    }).catch(() => setError('сервер fusionsim недоступний — працює лише ручне керування'))
  }, [])

  const note = useCallback((t: number, text: string) => {
    const a = { t, text }
    setActions((l) => [...l, a])
    setLog((l) => [a, ...l].slice(0, 60))
  }, [])

  const go = useCallback((ph: Phase, t: number, text: string, event = true) => {
    phaseRef.current = ph
    setPhase(ph)
    setPhaseText(text)
    if (event) setEvents((e) => [{ t, kind: ph, text }, ...e].slice(0, 50))
  }, [])

  const set = useCallback((ch: OverrideChannel, v: number | null) => {
    controls.getSim()?.shadow_start()          // the comparison starts at the first intervention
    controls.setOverride(ch, v)
    setValues((x) => {
      const n = { ...x }
      if (v === null) delete n[ch]
      else n[ch] = v
      return n
    })
  }, [controls])

  /** One update from the model: p, λ, shares and the fixer's chosen action. */
  const update = useCallback((prob: number, info: LiveInfo | null, s: Snapshot) => {
    setP(prob)
    setLam(info?.risk_total ?? null)
    setShares(info?.share_pct ?? null)
    if (info?.costs) setCosts(info.costs)
    if (s.t_trigger != null || s.disrupted) {
      if (phaseRef.current !== 'disrupted') go('disrupted', s.time, 'зрив почався — його вже не зупинити (передвісник → гасіння)')
      return
    }
    const c = cfg.current
    const mech = info?.share_pct ? dominantOf(info.share_pct) : null
    if (prob > c.threshold) {
      if (s.time > armedUntil.current && phaseRef.current !== 'applied') {
        go('alert', s.time, `p = ${prob.toFixed(2)}${mech ? `; причина: ${MECH_INFO[mech].short}` : ''}`)
      }
      armedUntil.current = s.time + c.armSec
    }
    const armed = s.time <= armedUntil.current
    const sim = controls.getSim()
    if (c.fixerOn && armed && sim && s.time - lastDecisionT.current >= c.period) {
      lastDecisionT.current = s.time
      const act = info?.action
      if (act && act !== 'none') {
        sim.shadow_start()                     // split the shadow *before* the change
        if (sim.fixer_apply_action(act)) {
          appliedAt.current = s.time
          setLastAction(act)
          const k = info?.costs ?? {}
          const text = `${ACTION_UA[act] ?? act} (ціна ${(k[act] ?? 0).toFixed(3)} проти ${(k.none ?? 0).toFixed(3)} без дії)`
          go('applied', s.time, text)
          note(s.time, text)
        }
      }
    }
    if (!armed && (phaseRef.current === 'applied' || phaseRef.current === 'alert')) {
      go(appliedAt.current !== null ? 'stable' : 'normal', s.time,
        appliedAt.current !== null ? `ризик знизився (p = ${prob.toFixed(2)}), розряд іде далі` : '')
    }
  }, [go, controls, note])

  const flush = useCallback(async () => {
    if (inFlight.current || queue.current.length === 0 || !cfg.current.connected) return
    inFlight.current = true
    try {
      if (newShot.current) await newShot.current.catch(() => {})
      const batch = queue.current.splice(0, queue.current.length)
      if (batch.length === 0) return
      const r = await api.liveStep(batch)
      const info = (r.info ?? []).filter(Boolean) as LiveInfo[]
      const last = info[info.length - 1] ?? null
      // Decisions use the instantaneous signal (p_now), not the running-maximum alarm.
      const pNow = last?.p_now ?? r.p[r.p.length - 1] ?? 0
      if (last) setNoAction(last.action === undefined)
      setPHist((h) => [...h, ...info.filter((_, i) => i % 3 === 0).map((x) => x.p_now ?? 0)].slice(-400))
      if (lastSnap.current) update(pNow, last, lastSnap.current)
    } catch (e) {
      setConnected(null)
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      inFlight.current = false
      if (queue.current.length) flush()
    }
  }, [update])

  /** Shadow plasma: follow it, pop up when it proves a disruption was prevented, and at the end. */
  const watchShadow = useCallback((s: Snapshot) => {
    const raw = controls.getSim()?.shadow_status_json()
    const realDisrupted = s.t_trigger != null
    const ended = s.status === 'Complete' || s.status === 'Disrupted'
    if (!raw) {
      // No intervention this pulse: still report how it ended.
      if (ended && !popped.current.summary) {
        popped.current.summary = true
        setPopup({ kind: 'summary', shadow: null, realDisrupted, realTrigger: s.t_trigger ?? null })
      }
      return
    }
    const sh = JSON.parse(raw) as ShadowStatus
    setShadow(sh)
    if (!popped.current.prevented && sh.t_trigger !== null && !realDisrupted) {
      popped.current.prevented = true
      setPopup({ kind: 'prevented', shadow: sh, realDisrupted: false, realTrigger: null })
    }
    if (ended && !popped.current.summary) {
      popped.current.summary = true
      setPopup({ kind: 'summary', shadow: sh, realDisrupted, realTrigger: s.t_trigger ?? null })
    }
  }, [controls])

  // Every physics step: feed the model, follow the shadow.
  useEffect(() => controls.subscribeSteps((snaps) => {
    const s = snaps[snaps.length - 1]
    if (!s) return
    if (s.time < lastTime.current) {
      // Reset / new pulse: start the story over.
      armedUntil.current = -Infinity; lastDecisionT.current = -Infinity; appliedAt.current = null
      popped.current = { prevented: false, summary: false }
      setActions([]); setEvents([]); setLog([]); setShadow(null); setPopup(null); setLastAction(null); setPHist([])
      setP(0); setLam(null); setShares(null); setCosts(null)
      go('normal', 0, '', false)
      // The model must start a fresh discharge, or it would carry the previous pulse's history.
      queue.current = []
      if (cfg.current.connected) {
        newShot.current = api.liveShot(s.duration ?? 20).finally(() => { newShot.current = null })
      }
    }
    lastTime.current = s.time
    lastSnap.current = s
    if (cfg.current.connected) {
      for (const x of snaps) if (x.channels) queue.current.push({ t: x.time, v: x.channels, extra: { scales: x.scales } })
      flush()
    } else if (s.t_trigger != null && phaseRef.current !== 'disrupted') {
      go('disrupted', s.time, 'зрив почався — його вже не зупинити (передвісник → гасіння)')
    }
    watchShadow(s)
  }), [controls, flush, go, watchShadow])

  const connect = async () => {
    setError(null)
    setConnecting(true)
    try {
      const r = await api.liveStart(model, 20)
      setConnected(r.model ?? model)
      setPHist([])
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setConnecting(false)
    }
  }
  const disconnect = async () => {
    setConnected(null)
    await api.liveStop().catch(() => {})
  }
  useEffect(() => () => { api.liveStop().catch(() => {}) }, [])

  const releaseAll = () => {
    controls.clearOverrides()
    controls.getSim()?.reset_scales()
    setValues({})
    note(snapshot?.time ?? 0, 'усі зміни скинуто — розряд знову йде за програмою')
  }

  const spark = pHist.length > 1
    ? pHist.map((v, i) => `${(i / (pHist.length - 1)) * 260},${40 - v * 40}`).join(' ') : ''
  const ui = PHASE_UI[phase]
  const sc = snapshot?.scales

  return (
    <>
      {/* ── Status banner: always on screen ── */}
      <div className="fixed left-1/2 -translate-x-1/2 top-12 z-[56] flex items-center gap-3 px-3 py-1.5 rounded-md border shadow-lg shadow-black/50 max-w-[62vw]"
        style={{ background: ui.bg, borderColor: ui.color }}>
        <span className="font-bold text-sm whitespace-nowrap" style={{ color: ui.color }}>{ui.label}</span>
        <span className="text-[11px] text-gray-100 truncate">{phaseText || (connected ? `модель ${connected}` : 'модель не під\'єднана — відкрийте «Керування»')}</span>
        <span className="font-mono text-[11px] tabular-nums text-gray-200 whitespace-nowrap">p {p.toFixed(2)}</span>
        {shadow && (
          <span className="text-[10px] font-mono px-1.5 rounded border whitespace-nowrap"
            style={shadow.t_trigger !== null ? { color: '#fca5a5', borderColor: '#f87171' } : { color: '#cbd5e1', borderColor: '#64748b' }}
            title="Тінь — та сама плазма з тими самими випадковостями, але без втручання fixer">
            без fixer: {shadow.t_trigger !== null ? `✖ ${shadow.t_trigger.toFixed(1)} с` : `ризик ${(shadow.p_shadow * 100).toFixed(0)} %`}
          </span>
        )}
        {events.length > 0 && (
          <span className="hidden xl:flex gap-1">
            {events.slice(0, 5).reverse().map((e, i) => (
              <span key={i} className="text-[9px] font-mono px-1 rounded" style={{ color: PHASE_UI[e.kind].color, border: `1px solid ${PHASE_UI[e.kind].color}` }}
                title={e.text}>{e.t.toFixed(1)}с</span>
            ))}
          </span>
        )}
      </div>

      {popup && <PopupView popup={popup} actions={actions} onClose={() => setPopup(null)} />}

      {/* ── Side panel ── */}
      <div className={`fixed right-2 top-14 bottom-2 z-[55] w-[360px] flex-col bg-[#0d1117]/95 border border-gray-700 rounded-lg shadow-2xl shadow-black/60 ${open ? 'flex' : 'hidden'}`}>
        <div className="flex items-center justify-between p-2 border-b border-gray-800">
          <div>
            <h2 className="panel-title">Керування розрядом</h2>
            <p className="text-[9px] text-gray-600">Модель стежить за плазмою, fixer змінює уставки до зриву — без паузи</p>
          </div>
          <button onClick={onClose} title="Сховати панель (усе продовжує працювати)" className="text-gray-500 hover:text-gray-300 cursor-pointer text-lg">✕</button>
        </div>
        <div className="flex-1 overflow-y-auto p-2 space-y-3">
          <section className="space-y-1.5">
            <span className="text-[10px] text-gray-500 uppercase tracking-wider">1. Модель</span>
            <select value={model} onChange={(e) => setModel(e.target.value)} disabled={!!connected}
              className="w-full bg-black border border-gray-700 rounded px-2 py-1 text-[11px] text-cyan-400 font-mono">
              {models.map((m) => <option key={m.id} value={m.id}>{m.folder}{m.name ? ` · ${m.name}` : ''}{m.problems.length ? ' ⚠' : ''}</option>)}
            </select>
            {connected ? (
              <button onClick={disconnect} className="w-full px-2 py-1 rounded text-[11px] bg-gray-700 hover:bg-gray-600 cursor-pointer">■ Від'єднати {connected}</button>
            ) : (
              <button onClick={connect} disabled={!model || connecting}
                className="w-full px-2 py-1 rounded text-[11px] bg-emerald-700 hover:bg-emerald-600 disabled:opacity-40 cursor-pointer">
                {connecting ? 'Під\'єднання… (модель завантажується)' : '▶ Під\'єднати модель (до Start)'}
              </button>
            )}
            {connected && noAction && (
              <p className="text-[9px] text-amber-400">Ця модель лише прогнозує: fixer нічого не змінюватиме. Для керування оберіть v8_fixer.</p>
            )}
            <div className="flex items-center gap-2">
              <div title="Сигнал зриву: імовірність зриву найближчим часом за моделлю">
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
            <span className="text-[10px] text-gray-500 uppercase tracking-wider">2. Fixer</span>
            <label className="flex items-center gap-2 cursor-pointer">
              <input type="checkbox" checked={fixerOn} onChange={(e) => setFixerOn(e.target.checked)} className="accent-amber-500" />
              <span className="text-[11px] text-gray-200">Fixer увімкнено: після сигналу p &gt; порогу змінює уставки сам</span>
            </label>
            <div className="grid grid-cols-3 gap-2">
              {([
                ['Поріг сигналу p', 'Вище цього p fixer починає діяти; нижчий поріг — більше часу до зриву', threshold, setThreshold, 0.05, 0, 1],
                ['Період, с', 'Як часто fixer перераховує дію, поки активний', period, setPeriod, 0.05, 0.1, 2],
                ['Активний після сигналу, с', 'Скільки fixer працює після останнього сигналу (щоб і повернути уставки)', armSec, setArmSec, 0.5, 0.5, 10],
              ] as [string, string, number, (v: number) => void, number, number, number][]).map(([label, hint, v, setv, step, min, max]) => (
                <label key={label} className="block" title={hint}><span className="text-[9px] text-gray-500">{label}</span>
                  <input type="number" min={min} max={max} step={step} value={v} onChange={(e) => setv(Number(e.target.value))}
                    className="w-full bg-black border border-gray-700 rounded px-1.5 py-0.5 font-mono text-[11px]" /></label>
              ))}
            </div>
            {costs ? (
              <div className="bg-gray-900 rounded p-1.5">
                <div className="text-[9px] text-gray-500 mb-1">Оцінка дій зараз: ціна = імовірність зриву + α·втрата продуктивності (менше — краще)</div>
                <table className="w-full text-[10px] font-mono tabular-nums">
                  <tbody>
                    {Object.entries(costs).sort((a, b) => a[1] - b[1]).map(([a, c], i) => (
                      <tr key={a} className={i === 0 ? 'text-amber-300' : 'text-gray-400'}>
                        <td className="font-sans">{i === 0 ? '▶ ' : ''}{ACTION_UA[a] ?? a}</td>
                        <td className="text-right">{c.toFixed(3)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                {lastAction && <div className="text-[9px] text-gray-500 mt-1">остання застосована дія: {ACTION_UA[lastAction] ?? lastAction}</div>}
              </div>
            ) : <p className="text-[9px] text-gray-600">Оцінки дій з'являться після під'єднання моделі й Start.</p>}
            {sc && (
              <div className="grid grid-cols-5 gap-1 text-[9px] font-mono text-gray-500" title="Множники fixer на програму розряду (1 = як заплановано)">
                <span>Iₚ×{sc.ip.toFixed(2)}</span><span>n̄ₑ×{sc.ne_target.toFixed(2)}</span>
                <span>D₂×{sc.d2_puff.toFixed(2)}</span><span>Ne×{sc.neon_puff.toFixed(2)}</span>
                <span>нагр×{sc.heating.toFixed(2)}</span>
              </div>
            )}
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
            <p className="text-[9px] text-gray-600">Помаранчевий повзунок — уставка змінена вручну; ✕ повертає програму.</p>
          </section>
          {error && <pre className="text-[10px] text-red-400 whitespace-pre-wrap">{error}</pre>}
        </div>
      </div>
    </>
  )
}
