# fusion-sim для хакатону: усе, що зроблено

Цей репозиторій — відкритий симулятор fusionsimulator.io (Daniel Burgess), на якому побудовано
інструмент організаторів `toksym`. Тут до нього додано **все, що є в toksym** (Dataset, Test run з
вибором моделі, звіт CAUGHT / FALSE ALARMS / THRESHOLD / MEDIAN WARNING, завантаження
`measured.csv` і `shots.csv`), але **без шуму приладів**, і **панель зміни параметрів під час
розряду** — вручну або автоматично за прогнозом моделі.

Дата: 2026-10-10. Робочі дані аналізу (не в репозиторії): `/home/alex/PlasmaHcktn/fusion_work/`.

---

Документація:
- [docs/USER_GUIDE.md](docs/USER_GUIDE.md) — **інструкція користувача** (з чого почати, усі панелі, як моделі віддають причини);
- [docs/PHYSICS_CONTROLS.md](docs/PHYSICS_CONTROLS.md) — **для фізиків**: керовані параметри, межі, механізми зриву, політика керування;
- [docs/FIXER.md](docs/FIXER.md) — **fixer**: запобігання зриву в реальному часі, датасет для моделі-учня;
- [SIM_API.md](SIM_API.md) — інтерфейс рушія для даних з діями, справжній ризик, швидкість.

### Журнал змін

**2026-10-11 (учень fixer)**
- Модель-учень `models/fixer_student`: вибирає дію fixer лише з 100 приладів (ImpactNet + тренди →
  16 регресорів HGB, вибір argmin P + α·втрата), навчена на вчителі + розмітці + раунді DAgger.
  На нових розрядах (seed 2000): зривів, яким можна запобігти, 31 → 5 — як у вчителя-оракула, W без втрат.
- `fusionsim teach --mode label | student | rules` (+ аудит учня вчителем); протокол: `model.extra`
  (множники fixer) і розширена відповідь з `action`; живий режим передає множники.
- Панель: «Вирішує оракул / учень», попередження, якщо модель не повертає дію.
- JET у браузері за замовчуванням на паливі DD (як у toksym), а не DT.

**2026-10-11 (fixer)**
- **Fixer** — запобігання зриву в реальному часі: після сигналу моделі копіює стан рушія, проганяє
  8 дій на 1 с вперед і застосовує найдешевшу за ціною «P(зрив) + α·втрата продуктивності»; діє через
  множники на програму (заплановані рампи зберігаються). Банер фаз над екраном і таблиця рішень у
  панелі «Керування». Опис — [docs/FIXER.md](docs/FIXER.md).
- `fusionsim teach` — вчитель на N сидах: `measured.csv` (+ множники, λ), `decisions.csv` (p/loss/cost
  кожної дії на кожному рішенні), `shots.csv`; `--mode none` — ті самі розряди без керування.
  Результат (seed 1000, 100 розрядів): зривів, яким можна запобігти, 24 → 2 при втраті W ≈ 0,7 %.
- Ядро: `Simulation: Clone`, `ProgramScales`, режим копії (без жеребу; розклад машини за планом,
  газ — утримується), `ShotSpec::plan` (програма без збурень), WASM-методи fixer.

**2026-10-11**
- Порт за замовчуванням — **9999** (`fusionsim serve`, проксі Vite, документація).
- Виправлено запуск моделей з відносним `--python` (напр. `../participant/venv/bin/python`): модель
  стартує з власної теки, тож шлях тепер робиться абсолютним на старті (без розкриття symlink venv,
  інакше губився б Python із numpy). Те саме для `--models-dir`.
- **Причини зриву.** Протокол моделі розширено сумісно з toksym: сервер шле `"rich": true`, і модель
  може відповісти JSON `{"p", "p_now", "risk_total", "share_pct"}`. Залік зберігає справжній розклад
  λ рушія в момент жереба й розклад моделі в піку її λ; у звіті — блок «Чинники зриву» (точність
  головного механізму, таблиця, середні частки), у рядках розрядів — причина за моделлю і ✓/✗;
  `measured.csv` прогону — колонки `model_lambda, share_*`.
- Нові здачі в `../models`: `impactnet` (ImpactNet: p = 1 − e^(−λ·1 с)) і `v8_impact` (тривога v8 +
  причини ImpactNet). На 30 розрядах (seed 3) головний механізм вгадано в 5 з 6 зривів.
- Панель **Керування**: живі частки механізмів від моделі, порадник із кнопкою «Застосувати»,
  автокерування за механізмом моделі (за малої λ — за найближчою межею), рішення за миттєвим `p_now`.
- **Інтерфейс українською**: Керування, Датасет, Залік.

**2026-10-10** — первинна версія: JET як у toksym, 100 каналів, генератор, залік, сервер, панелі, overrides.

## 1. Швидкий старт

```bash
# один раз: Rust (rustup у ~/.cargo), wasm-pack, npm-залежності
export PATH=$HOME/.cargo/bin:$PATH        # у fish: fish_add_path ~/.cargo/bin
./build.sh                                # WASM + fusionsim + web/dist

# сервер з інтерфейсом, моделями з ../models і Python, де є numpy
./target/release/fusionsim serve \
    --models-dir /home/alex/PlasmaHcktn/models \
    --python /home/alex/PlasmaHcktn/participant/venv/bin/python
# → http://127.0.0.1:9999/   (JET → H-mode → Run)
```

Угорі праворуч у control room три кнопки:

| кнопка | що робить |
|---|---|
| 🎛 **Керування** | прогноз зриву (λ рушія або ваша модель), частки механізмів, порадник, автокерування, 10 уставок вручну |
| ⛁ **Датасет** | розрядів, Master seed, частота запису, колонки істини → ↓ measured.csv, ↓ shots.csv |
| ⏱ **Залік** | модель (список з `--models-dir` або перетягнута тека), lead, бюджет хибних тривог, розрядів, seed → плитки Спіймано / Хибні тривоги / Поріг / Медіанне випередження, «Чинники зриву», рядки розрядів, ↓ measured.csv (+p, частки), ↓ shots.csv, ↓ run.json, перерахунок, відкриття run.json |

Режим розробки фронтенду: `fusionsim serve …` в одному терміналі, `cd web && npm run dev` в
іншому (Vite проксіює `/api` на `127.0.0.1:9999`).

### Командний рядок (як `toksym …`)

```bash
fusionsim gen --shots 200 --seed 1 --out mydata [--stride 4] [--extras program,transport,physics,risk]
fusionsim check ../models/submission_v8
fusionsim score --submission ../models/submission_v8 --shots 200 --seed 1 \
                --lead 0.1 --budget 0.05 --report run.json --save-data rundata
fusionsim rescore run.json --lead 0.3 --budget 0.1
fusionsim teach --shots 100 --seed 2000 --mode teacher|label|student|rules|none [--student ../models/fixer_student] --out DIR
```

`teach` — fixer і дані для моделі-учня, див. [docs/FIXER.md](docs/FIXER.md). Моделі на sklearn запускайте з
`OMP_NUM_THREADS=1` і `--python ../fusion_work/venv/bin/python` (там є scikit-learn).

Python моделі: `--python ШЛЯХ` або змінна `FUSIONSIM_PYTHON` (за замовчуванням `python3`).
Моделі — той самий формат здачі, що в toksym (`toksym.json` + `model.py` + ваги); обгортка
`toksym_model.py` підкладається автоматично (`crates/fusion-cli/toksym_model.py`).

---

## 2. Порівняння з симулятором організаторів (toksym)

Як аналізувалось: `toksym-linux ui` віддає свій веб-інтерфейс — це **той самий фронтенд
fusion-sim** з доданими панелями; його бандл розібрано (API, тексти, поля). У бінарнику видно
структуру коду: `tok-sym-core/{channels,noise}.rs` і `toksym-cli/{gen,scoring,server,model,rescore,stream}.rs`.
Конфіг шуму вшитий у бінарник у JSON — підставивши **конфіг з нулями** (`--noise-config`),
toksym видає чисті канали; з колонками істини (`--extras`) з них виведено формули.

| що | toksym | тут |
|---|---|---|
| фізичне ядро | змінений tok-sym-core | той самий код + JET приведено до toksym (нижче) |
| JET | R0 2.89, a 0.95, κ 1.70, δ 0.25, V 100, S 150, z0 0.1, утримання ×1.0 | **так само** (`devices.rs`) — у fusion-sim було 2.85 / 0.80 / 1.95 / 0.20 / 80 / 120 / ×1.35 |
| пресети JET | H-mode 3.5 МА, n = 0.7 n_GW, NBI 25 + ICRH 4 | збігаються автоматично після зміни пристрою |
| 100 каналів | `channels.rs` (закритий) | `crates/tok-sym-core/src/channels.rs` — ті самі назви й порядок |
| шум, відмови, «завислі» прилади | `noise.rs` | **немає навмисно** (вимога задачі) |
| генератор розрядів | `gen.rs` (закритий) | `crates/tok-sym-core/src/generator.rs` — та сама родина розподілів |
| залік | `scoring.rs` | `crates/fusion-cli/src/scoring.rs` — ті самі правила |
| сервер / UI | `/api/*` + панелі | ті самі ендпоінти й панелі + `/api/models`, `/api/live/*`, панель Control |
| живе керування | немає | `ProgramOverrides` у ядрі, панель 🎛, `tools/closed_loop.py` |

### 2.1 Перевірка фізики
Програми розрядів toksym (з чистого датасету) прогнано через наш рушій крок за кроком
(`fusion_work/replay.py`): I_p, n̄_e, P_in, P_rad, частка домішок і моменти L→H / H→L
збігаються; W_th і T_e0 — у межах ~3 % (різниця від фази ELM і невідомих κ, δ, яких немає в
даних toksym). Підбір коефіцієнта утримання: ×1.0 дає медіанне W_ours/W_toksym = 0.97–1.00,
×1.35 (старе значення fusion-sim) — 1.22–1.32. Формула неонового випромінювання збігається
до 0.1 %.

### 2.2 Канали (`channels.rs`)
Порівняння на 6 відтворених розрядах: **56 із 100 каналів — медіанна похибка < 2 %**, ще ~35 —
2–5 % (хорди й профілі, через невідому κ). Наближені (регресія по даних toksym):

| канали | як рахується | точність |
|---|---|---|
| mag_ip, mag_bt, mag_diamag (=W), mag_vloop (=P_Ω/I_p), coil_tf_current (18.2·B_t), coil_tf_voltage, coil_pf_radial (1.793·I_p), psu_dclink (3300 − 4·I_p), coil_tf_temp, cryo_pressure, gas_*, vac_rga_*, nbi/ich/ech, nbi_source_*, ich_antenna, prog_*, neutron_* | точні формули | ~0 % |
| ece_01…16 | T_e(ρ = 0.064·(k−1)) з профілю | ~0 % (+ похибка профілю) |
| ifr_chord_* | вертикальні хорди ∫n_e ds × κa, ρ_imp = 0.16 / 0.41 / 0.41 / 0.71 / 0.87 | 2–4 % |
| sxr_01…08 | хорди ∫n_e²√T_e ds, ρ_imp = 0.71 / 0.50 / 0.31 / 0.06 (дзеркально) | 3–5 % |
| bolo_01…08 | P_rad розподілено по хордах ε ∝ n_e²/√T_e, сума = P_rad | 2–6 % |
| dalpha_* | 0.5·n̄_e (midplane 0.175), ×8 Type I / ×2.5 Type II ELM | точно |
| mag_saddle_lm | амплітуда locked mode (0 до жереба, e^{20t}) | точно |
| mag_mirnov_k | передвісник: lm·sin(50t + (k−1)π/4); ELM: 0.16·sin((k−1)·3π/4) | форма відтворена |
| mag_halo_* | 0.35 / 0.245 кА на МА після жереба | точно |
| mag_flux_loop_* | степеневі фіти по I_p, W | 0.4–2 % |
| coil_pf_shape/vert | степеневі фіти по I_p, W, l_i, B_t | 12–18 % (залежать від κ, δ) |
| div_tc_*, wall_tc_* | релаксація τ = 0.31 с до 120+40k + 16.8·0.85^k·P_sol | 4–7 % (без ELM-сплесків) |
| div_probe_isat_*, vac_pressure | степеневі фіти (n̄_e, P_sol, D₂) | 20–35 % |
| coil_cs_flux | −(1.77·I_p + 0.41·∫V_loop dt) | грубо (~35 %) |

### 2.3 Генератор розрядів (`generator.rs`)
Оцінено на 200 розрядах toksym (seed 7, без шуму):
- базова H-mode-програма JET, **усі вузли × duration/20**, тривалість U(12, 20) с;
- плато: B_t 2–3.4 Т, I_p 1.8–4 МА, n 0.3–1.5, NBI 4–34, ECH 0–6, ICRH 0–9 МВт, D₂ 0.3–7
  (плато — половина стартового піку), неон 0–0.7, κ 1.55–1.68, δ 0.2–0.4;
  обмеження **q95 ≥ 3.2** (через I_p) і **f_GW ≤ 0.72** (через n);
- вікна: NBI 4→5…15→16, ICRH 3.6→4…15→16, ECH і неон 6→7…14→15 (× масштаб);
- збурення стартують на 24–70 % плато; 1/5 розрядів без збурень, ~1/6 з двома;
  частоти heat_loss 0.32, neon 0.29, low_q 0.22, density 0.16; ~20 % повертаються назад:
  - **density**: n → U(0.6, 1.7)·n_GW, рампа 0.5–2.9 с, **тримається до кінця імпульсу**;
  - **neon**: засів → U(0.3, 2.2), рампа до 2.9 с, **тримається до кінця** (тому після вимкнення
    нагріву P_rad/P_in > 1 — головна причина радіаційних зривів у toksym);
  - **low_q**: I_p → рівень, де q95 = U(2.0, 3.1), ≤ 5 МА, рампа 0.4–2.8 с; спад від нового рівня;
  - **heat_loss**: NBI падає до 2.5–47 % за 0.02–0.3 с.
- Результат (400 розрядів): **27 % зривів** (toksym: 29 %), спонтанних 6 % (toksym ~4 %);
  за типом density 0.48 (0.41), heat_loss 0.16 (0.09), low_q 0.31 (0.38), neon 0.43 (0.61).
- Випадкові потоки інші, ніж у toksym: той самий seed дає **інші** розряди, але з того самого
  розподілу.

### 2.4 Запис і мітки (як у toksym)
- рядок на кожен крок 5 мс; запис зривного розряду закінчується **до** кроку термічного гасіння;
- `shots.csv` датасету: `shot_id, seed, master_seed, scenario, duration_s, disrupted,
  t_trigger_s, t_tq_s, spontaneous, wall_contact, n_steps, t_end_s`;
- **spontaneous** = λ у момент жереба < 0.02 с⁻¹ (біля підлоги ≈ 0.006) — наша межа, у toksym
  точний поріг невідомий;
- колонки істини: групи toksym `program, transport, pedestal, impurity, equilibrium, profiles` +
  наші `physics` (q95, β_N, f_GW, P_rad/P_in) і `risk` (λ і чотири сигмоїди).

### 2.5 Залік (`scoring.rs`) — правила toksym
- модель отримує `meta = {shot_id, dt, duration, scenario}` і 100 каналів щокроку (протокол 1:
  hello/ready, shot_start, step → число, shot_end, bye; `#` — лог; 30 с на ready, 1 с на крок);
- тривога — перший крок, де **p > поріг**; звіт зберігає «сходинки» нових максимумів p;
- вердикти: caught (t_Q − t_alarm ≥ lead), late, missed, false_alarm, quiet, excluded
  (спонтанний, контакт зі стінкою, гасіння не записане);
- TPR = caught / незвільнені зриви; FPR = хибні / розряди без зриву;
- поріг підбирається: серед порогів (середини між сусідніми значеннями p) з FPR ≤ бюджету —
  найбільший TPR, при рівності — найнижчий поріг; якщо TPR = 0 → поріг 1.0;
  медіанне випередження — по caught;
- `run.json` має ті самі поля, що й toksym (`summary, curve{lead_times, curves, reference},
  rule, timing, shots`), тож його відкриває і наш, і їхній «open a saved run.json».

---

## 3. Зміна параметрів під час симуляції

### 3.1 Ядро
`crates/tok-sym-core/src/simulation.rs`:
- `ProgramOverrides` — для кожного з 10 каналів (`ip, bt, ne_target, p_nbi, p_ech, p_ich, kappa,
  delta, d2_puff, neon_puff`) `Option<f64>`, що з наступного кроку замінює значення програми;
- `Simulation::set_override(channel, value)`, `clear_override(channel)`, `effective_values(t)`;
- спад струму визначається за **запланованою** I_p, тож зниження I_p контролером не вимикає жереб;
- знімок містить `overrides`, `channels` (100), `risk_*`, `t_trigger`, `t_tq`, `prog_kappa/delta`.

WASM (`SimHandle`): `set_override`, `clear_override`, `clear_overrides`, `overrides_json`,
`set_program_json`, `seed`. React-хук: `controls.setOverride`, `clearOverrides`,
`subscribeSteps(cb)` (кожен крок фізики).

### 3.2 Панель 🎛 Control (`web/src/components/ControlPanel.tsx`)
- 10 слайдерів (помаранчевий — override активний, ✕ — назад до програми), «↺ back to program»;
- предиктор: **Engine λ** (p = 1 − e^{−λ·1 с}, «оракул», працює без сервера) або **Your model**
  (модель з `--models-dir` отримує ті самі 100 каналів через `/api/live/*`);
- **Auto-control**: коли p > поріг (не частіше ніж раз на «Min. time between actions»), береться
  найближча межа і застосовується дія:

| межа | ознака | дія |
|---|---|---|
| Грінвальд | f_GW | n̄_e target × 0.85, D₂ × 0.5 |
| кінк | 2.2 / q95 | I_p ↓ до q95 ≈ 3 |
| радіація | P_rad / P_in | неон → 0, NBI +5 МВт |
| Тройон | β_N / 2.8 | NBI × 0.8 |

  Свою політику — у функції `react()` цього файла. Журнал дій — унизу панелі.

### 3.3 Без браузера: `tools/closed_loop.py`
Модель у форматі здачі імпортується напряму; рушій — `headless` з програмою
`generated:SEED:SHOT` (той самий розряд, що в `fusionsim gen/score`). Причина визначається лише
з **вимірюваних каналів** (f_GW з хорди, q ∝ B_t/I_p, P_rad = Σ болометрів).

```bash
PY=/home/alex/PlasmaHcktn/participant/venv/bin/python
$PY tools/closed_loop.py --model ../models/submission_v8 --shots 40 --seed 3 --no-control   # 11 / 40 зривів
$PY tools/closed_loop.py --model ../models/submission_v8 --shots 40 --seed 3 --threshold 0.25  # 5 / 40
```

Нюанс: submission_v8 повертає **бігучий максимум** p — після дії він не падає, і контролер
продовжує «закручувати» I_p / густину кожні 0.3 с. Для керування модель має віддавати миттєве p
(або вимикайте max у її постобробці).

Python-клієнт рушія — `tools/fusion_sim_client.py` (`step`, `set`, `clear`, `snapshot`, `program`).

---

## 4. HTTP API (`fusionsim serve`)

| метод і шлях | що |
|---|---|
| GET `/api/info` | dt, eval_shots, eval_seed, python, models_dir, групи extras |
| GET `/api/models` | локальні здачі з `--models-dir` + проблеми перевірки |
| POST `/api/model/reset`, POST `/api/model/file?path=` , GET `/api/model/list` | завантаження теки (як toksym) |
| POST `/api/dataset` `{shots, seed, stride, extras}` · GET `/progress` · GET `/file?name=measured.csv\|shots.csv` · POST `/cancel` | датасет |
| POST `/api/validate` `{model, lead_time_s, fpr_budget, shots, seed}` · GET `/progress` · GET `/file?name=measured.csv\|shots.csv\|run.json` · POST `/cancel` | test run |
| POST `/api/rescore` `{report, lead_time_s, fpr_budget}` | перерахунок звіту |
| POST `/api/live/start` `{model, duration}` · POST `/api/live/step` `{steps:[{t, v, extra?}]}` → `{p:[…], info:[…]}` · POST `/api/live/stop` | модель у реальному часі (`extra` → `model.extra`, напр. множники fixer; `info` — розширена відповідь: p_now, λ, частки, action, costs) |

`model` — шлях до теки здачі або `"uploaded"` (перетягнута тека). Файли пишуться в `--out`
(за замовчуванням `./fusionsim-data`).

---

## 5. Що змінено у файлах

| файл | зміна |
|---|---|
| `crates/tok-sym-core/src/devices.rs` | JET як у toksym |
| `crates/tok-sym-core/src/simulation.rs` | overrides, канали в знімку, хронологія зриву, `compute_geometry`; `Clone`, `ProgramScales` (множники fixer), режим копії (`stochastic`, `compute_channels`, `freeze_program_at`, `plan`), `risk()` |
| `crates/tok-sym-core/src/fixer.rs` | **новий** — fixer: дії, прогін копій, ціна, покроковий `Planner` |
| `crates/tok-sym-core/src/diagnostics.rs` | `NoiseGen: Clone` |
| `crates/tok-sym-core/src/disruption.rs` | `locked_mode_amplitude()` без шуму |
| `crates/tok-sym-core/src/channels.rs` | **новий** — 100 каналів |
| `crates/tok-sym-core/src/generator.rs` | **новий** — генератор розрядів; `ShotSpec::plan` (програма без збурень) |
| `crates/tok-sym-core/src/wasm_api.rs` | override-методи, `seed`; fixer: `fixer_begin/step/progress/result`, `fixer_apply_action`, `set_scales_json`, `reset_scales` |
| `crates/tok-sym-core/src/bin/headless.rs` | **новий** — рушій по JSON-рядках (stdin/stdout) |
| `crates/fusion-cli/` | **новий** — `fusionsim`: gen, check, score, rescore, serve, teach |
| `crates/fusion-cli/src/teach.rs` | **новий** — `fusionsim teach` (teacher / label / student / rules / none) |
| `crates/fusion-cli/src/model.rs`, `toksym_model.py` | розширена відповідь моделі (`rich`), `step_extra` → `model.extra` (множники fixer) |
| `web/src/components/{DatasetPanel,TestRunPanel,ControlPanel}.tsx` | **нові** панелі (українською) |
| `web/src/lib/mechanisms.ts` | **новий** — механізми зриву: назви, кольори, частки |
| `crates/fusion-cli/src/mech.rs` | **новий** — справжній розклад λ рушія на механізми |
| `docs/USER_GUIDE.md`, `docs/PHYSICS_CONTROLS.md`, `docs/FIXER.md` | **нові** — інструкція користувача, довідка для фізиків, fixer і модель-учень |
| `web/src/lib/api.ts` | **новий** — клієнт API |
| `web/src/lib/useSimulation.ts`, `types.ts`, `pages/ControlRoom.tsx` | overrides, підписка на кроки, `getSim()`, кнопки; JET за замовчуванням на DD |
| `web/src/components/ControlPanel.tsx` | fixer (оракул / учень), банер фаз, таблиця рішень, простий порадник, ручні уставки |
| `web/vite.config.ts`, `build.sh`, `.gitignore` | проксі `/api`, збірка `fusionsim` |
| `tools/fusion_sim_client.py`, `tools/closed_loop.py` | Python-клієнт і замкнений контур |

Тести: `cargo test --release` — 58 тестів ядра (у т.ч. генератор і fixer: копія не змінює оригінал; зниження густини знижує ризик Грінвальда) + залік + аудит фізики, зелені.

Поза репозиторієм: моделі `../models/{impactnet,v8_impact,fixer_student}` (здачі), навчання учня й дані — `../fusion_work/{student,fixer_data}`.

### Виправлені пастки
- xorshift з малим seed (1, 7, 42) давав rand ≈ 0 → зрив на першому кроці при I_p > 0.24 МА;
  seed тепер перемішується splitmix64 (headless, WASM `seed()`, `fusionsim`);
- діагностика fusion-sim додає до locked mode шум для відображення — канал сідлової котушки
  бере чисту амплітуду;
- відсутні статичні файли (скрипт аналітики Vercel) сервер віддає 404, а не index.html.

---

## 6. Обмеження, які варто пам'ятати

1. **Без шуму** — моделі, навчені на шумних даних toksym, тут бачать «ідеальні» прилади; пороги
   й якість не переносяться 1:1 (v8 на 40 наших розрядах: 5/7 caught при FPR 3 %, на toksym —
   18/18 при 5 %; вибірки малі).
2. Розряди з тим самим seed **не ті самі**, що в toksym; розподіл той самий з точністю ±5–10 п.
3. κ і δ у даних toksym не видно — канали, що від них залежать (PF-котушки, CS-потік, зонди
   дивертора, тиск), наближені.
4. Межа «спонтанного» зриву (λ < 0.02 с⁻¹) — наш вибір.
5. Панель Control діє на інтерактивний розряд у браузері (WASM); той самий контур без UI —
   `tools/closed_loop.py`.
