// Disruption mechanisms as the engine and ImpactNet split λ:
// share_k = C_k / ΣC with C_k = −ln(1 − r_k^p) (p = 3 for limits, 1 for locked mode).

export const MECHANISMS = ['greenwald', 'q95', 'radiation', 'troyon', 'other'] as const
export type Mechanism = (typeof MECHANISMS)[number]

export const MECH_INFO: Record<Mechanism, { label: string; short: string; color: string; hint: string }> = {
  greenwald: { label: 'Грінвальд (густина)', short: 'Грінвальд', color: '#38bdf8', hint: 'n̄ₑ наближається до межі Грінвальда n_GW = Iₚ/πa²' },
  q95: { label: 'Кінк (низький q95)', short: 'q95', color: '#f59e0b', hint: 'Iₚ завеликий відносно Bₜ і форми: q95 → 2.4' },
  radiation: { label: 'Радіація', short: 'Радіація', color: '#f472b6', hint: 'P_rad/P_in → 0.8: неон або втрата нагріву' },
  troyon: { label: 'Тройон (βN)', short: 'Тройон', color: '#a78bfa', hint: 'βN → 2.4: забагато тиску на струм' },
  other: { label: 'Інші (locked mode, фон)', short: 'Інші', color: '#94a3b8', hint: 'Locked mode і фонова підлога λ — при малій λ частки нічого не значать' },
}

export type Shares = Partial<Record<Mechanism, number>>

export interface CauseInfo {
  share_pct: Shares
  risk_total?: number | null
  dominant?: Mechanism | null
}

/** Mechanism with the largest share. */
export function dominantOf(s: Shares | undefined | null): Mechanism | null {
  if (!s) return null
  let best: Mechanism | null = null
  let v = -1
  for (const m of MECHANISMS) {
    const x = s[m] ?? 0
    if (x > v) { v = x; best = m }
  }
  return best
}
