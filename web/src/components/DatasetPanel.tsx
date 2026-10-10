import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type DatasetResult } from '../lib/api'

// Plateau ranges the generator draws from (crates/tok-sym-core/src/generator.rs).
const RANGES: [string, number, number, string, number][] = [
  ['Bₜ — тороїдальне поле', 2, 3.4, 'Т', 2],
  ['Iₚ на плато', 1.8, 4, 'МА', 2],
  ['Уставка густини', 0.3, 1.5, '10²⁰m⁻³', 2],
  ['Потужність NBI', 4, 34, 'МВт', 1],
  ['Потужність ECH', 0, 6, 'МВт', 1],
  ['Потужність ICRH', 0, 9, 'МВт', 1],
  ['Напуск D₂', 0.3, 7, '10²⁰/с', 2],
  ['Засів неону', 0, 0.7, '10²⁰/с', 3],
  ['Витягнутість κ', 1.55, 1.68, '', 2],
  ['Трикутність δ', 0.2, 0.4, '', 2],
]

const EXTRAS: { id: string; label: string; hint: string; n: number; ours?: boolean }[] = [
  { id: 'program', label: 'Уставки програми', hint: 'Що програма розряду задає в цю мить (разом з κ, δ)', n: 8 },
  { id: 'transport', label: 'Транспорт і енергобаланс', hint: 'Струм, температура, густина, енергія, утримання', n: 11 },
  { id: 'pedestal', label: 'H-mode, педестал, ELM', hint: 'Режим утримання, вершина педесталу, стан ELM', n: 7 },
  { id: 'impurity', label: 'Домішки й паливо', hint: 'Частка домішок, напуск D₂, засів неону', n: 3 },
  { id: 'equilibrium', label: 'Геометрія рівноваги', hint: 'Магнітна вісь, X-точка, ознака limited', n: 5 },
  { id: 'profiles', label: 'Радіальні профілі', hint: 'Профілі Te і ne по 51 точці — 102 колонки, основна вага файлу', n: 102 },
  { id: 'physics', label: 'Запаси стійкості', hint: 'q95, βN, f_GW, P_rad/P_in — входи формули ризику', n: 4, ours: true },
  { id: 'risk', label: 'Інтенсивність зриву λ', hint: 'λ, чотири сигмоїди rᵢ і член locked mode — «відповідь» у явному вигляді', n: 6, ours: true },
]

const STRIDES: [number, string][] = [[1, '5 мс'], [2, '10 мс'], [4, '20 мс'], [10, '50 мс'], [20, '100 мс'], [100, '500 мс']]

function NumField({ label, value, min, max, disabled, onChange }: {
  label: string; value: number; min: number; max: number; disabled: boolean; onChange: (v: number) => void
}) {
  return (
    <div>
      <label className="text-[10px] text-gray-500 uppercase tracking-wider">{label}</label>
      <input
        type="number" value={value} min={min} max={max} step={1} disabled={disabled}
        onChange={(e) => {
          const v = parseFloat(e.target.value)
          if (Number.isFinite(v)) onChange(Math.round(Math.min(max, Math.max(min, v))))
        }}
        className="mt-1 w-full bg-gray-800 border border-gray-700 rounded px-2 py-1 text-xs text-cyan-400 font-mono focus:outline-none focus:border-cyan-600 disabled:opacity-50"
      />
    </div>
  )
}

export default function DatasetPanel({ onClose }: { onClose: () => void }) {
  const [runs, setRuns] = useState(20)
  const [seed, setSeed] = useState(1)
  const [stride, setStride] = useState(1)
  const [extras, setExtras] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [done, setDone] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<DatasetResult | null>(null)
  const cancelled = useRef(false)

  const columns = 102 + EXTRAS.filter((e) => extras.includes(e.id)).reduce((a, e) => a + e.n, 0)
  const rows = Math.ceil(16 / 0.005 / stride) * runs

  const generate = useCallback(async () => {
    setBusy(true); setError(null); setResult(null); setDone(0); cancelled.current = false
    try {
      await api.dataset({ shots: runs, seed, stride, extras })
      for (;;) {
        await new Promise((r) => setTimeout(r, 300))
        const p = await api.datasetProgress()
        setDone(p.done)
        if (p.error) throw new Error(p.error)
        if (!p.running) {
          if (!p.result) throw new Error('збірка скінчилась без результату')
          setResult(p.result)
          break
        }
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }, [runs, seed, stride, extras])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && !busy) onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [busy, onClose])

  const pctDone = runs > 0 ? Math.round((done / runs) * 100) : 0

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/70 p-4" onClick={busy ? undefined : onClose}>
      <div className="w-full max-w-2xl max-h-full flex flex-col bg-[#0d1117] border border-gray-700 rounded-lg shadow-2xl shadow-black/60"
        onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between p-3 border-b border-gray-800">
          <div>
            <h2 className="panel-title">Збірка датасету</h2>
            <p className="text-[9px] text-gray-600 mt-0.5">Рушій проганяє розряди без інтерфейсу й записує потрібні колонки — прилади без шуму</p>
          </div>
          <button onClick={onClose} disabled={busy} className="text-gray-500 hover:text-gray-300 cursor-pointer text-lg disabled:opacity-30">✕</button>
        </div>

        <div className="flex-1 overflow-y-auto p-3 space-y-4">
          <div className="grid grid-cols-3 gap-3">
            <NumField label="Розрядів" value={runs} min={1} max={2000} disabled={busy} onChange={(v) => { setRuns(v); setResult(null) }} />
            <NumField label="Master seed" value={seed} min={0} max={2 ** 31 - 1} disabled={busy} onChange={(v) => { setSeed(v); setResult(null) }} />
            <div>
              <label className="text-[10px] text-gray-500 uppercase tracking-wider">Записувати кожні</label>
              <select value={stride} disabled={busy} onChange={(e) => { setStride(parseInt(e.target.value, 10)); setResult(null) }}
                className="mt-1 w-full bg-gray-800 border border-gray-700 rounded px-2 py-1 text-xs text-cyan-400 font-mono focus:outline-none disabled:opacity-50">
                {STRIDES.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
              </select>
            </div>
          </div>

          <div>
            <label className="text-[10px] text-gray-500 uppercase tracking-wider">Що змінюється між розрядами</label>
            <p className="text-[9px] text-gray-600 mt-0.5 mb-1.5">
              Фіксовано, як і склад колонок, — та сама родина розрядів, що в toksym організаторів. Ви обираєте лише скільки розрядів і як часто записувати.
            </p>
            <div className="grid grid-cols-2 gap-x-4 gap-y-0.5">
              {RANGES.map(([label, lo, hi, unit, prec]) => (
                <div key={label} className="flex items-baseline justify-between gap-2">
                  <span className="text-[11px] text-gray-300 truncate">{label}</span>
                  <span className="font-mono text-[10px] text-gray-500 tabular-nums whitespace-nowrap">
                    {lo.toFixed(prec)} … {hi.toFixed(prec)}{unit && <span className="text-gray-600"> {unit}</span>}
                  </span>
                </div>
              ))}
            </div>
            <p className="text-[9px] text-gray-600 mt-1.5">
              Струм і густина обмежені, щоб жоден розряд не стартував за межею стійкості: q₉₅ ≥ 3.2 і густина ≤ 0.72 межі Грінвальда.
              Збурення (density — густина, neon — неон, low_q — струм, heat_loss — втрата нагріву) стартують у випадковий момент плато; п'ята частина розрядів без збурень, шоста — з двома.
            </p>
          </div>

          <div>
            <label className="text-[10px] text-gray-500 uppercase tracking-wider">Колонки файлу</label>
            <div className="flex items-baseline justify-between gap-2 py-0.5 opacity-60 mt-1">
              <span className="text-[11px] text-gray-300">100 вимірюваних каналів + номер розряду і час</span>
              <span className="font-mono text-[10px] text-gray-500 tabular-nums">102</span>
            </div>
            <p className="text-[9px] text-gray-600 mt-2 mb-1">
              Ті самі назви й порядок, що в toksym, але без шуму й відмов. ⚠️ На заліку модель отримує <em>лише</em> їх; колонки нижче — справжній стан плазми для навчання й аналізу.
            </p>
            <div className="grid grid-cols-2 gap-x-3 gap-y-0.5">
              {EXTRAS.map((g) => (
                <label key={g.id} title={g.hint} className="flex items-start gap-1.5 py-0.5 cursor-pointer">
                  <input type="checkbox" checked={extras.includes(g.id)} disabled={busy}
                    onChange={() => { setExtras((x) => (x.includes(g.id) ? x.filter((c) => c !== g.id) : [...x, g.id])); setResult(null) }}
                    className="accent-cyan-500 mt-0.5 cursor-pointer" />
                  <span className="min-w-0">
                    <span className="text-[11px] text-gray-300">{g.label}</span>
                    <span className="text-[10px] text-gray-600 ml-1 tabular-nums">({g.n})</span>
                    {g.ours && <span className="text-[9px] text-amber-500/80 ml-1">fusion-sim</span>}
                    <span className="block text-[9px] text-gray-600 leading-tight">{g.hint}</span>
                  </span>
                </label>
              ))}
            </div>
          </div>

          <div className="text-[10px] font-mono text-gray-500 border-t border-gray-800 pt-2 space-y-0.5">
            <div>≈ {rows.toLocaleString()} рядків × {columns} колонок</div>
            <div>Тривалість 12–20 с (випадкова) · паливо DD · нижня X-точка · L→H перехід вирішує рушій</div>
          </div>

          {busy && (
            <div className="space-y-1">
              <div className="h-1.5 bg-gray-800 rounded overflow-hidden">
                <div className="h-full bg-cyan-500 transition-[width] duration-150" style={{ width: `${pctDone}%` }} />
              </div>
              <div className="text-[10px] font-mono text-gray-500">розряд {done}/{runs}</div>
            </div>
          )}
          {error && <div className="text-[11px] text-red-400 font-mono">Помилка: {error}</div>}
          {result && !busy && (
            <div className="text-[10px] font-mono text-emerald-400 space-y-0.5">
              <div>{cancelled.current ? 'Зупинено — частковий датасет: ' : 'Готово: '}{result.rows.toLocaleString()} рядків × {result.columns} колонок · записано в {result.dir}</div>
              <div className="text-gray-500">
                зірвалось {result.disrupted} з {result.shots}{result.shots > 0 && ` (${((result.disrupted / result.shots) * 100).toFixed(0)}%)`}
              </div>
            </div>
          )}
        </div>

        <div className="p-3 border-t border-gray-800 flex gap-2">
          {result && !busy ? (
            <>
              <a href={api.datasetFile('measured.csv')} download
                className="flex-1 text-center px-4 py-2 bg-cyan-600 hover:bg-cyan-500 rounded text-sm font-bold">↓ measured.csv</a>
              <a href={api.datasetFile('shots.csv')} download
                className="flex-1 text-center px-4 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm font-bold">↓ shots.csv</a>
              <button onClick={() => setResult(null)} className="px-3 py-2 bg-gray-800 hover:bg-gray-700 rounded text-sm cursor-pointer">↺</button>
            </>
          ) : busy ? (
            <button onClick={() => { cancelled.current = true; api.datasetCancel() }}
              className="flex-1 px-4 py-2 bg-amber-700 hover:bg-amber-600 rounded text-sm font-bold cursor-pointer">■ Зупинити після цього розряду</button>
          ) : (
            <button onClick={generate} className="flex-1 px-4 py-2 bg-cyan-600 hover:bg-cyan-500 rounded text-sm font-bold cursor-pointer">
              ▶ Згенерувати {runs} розрядів
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
