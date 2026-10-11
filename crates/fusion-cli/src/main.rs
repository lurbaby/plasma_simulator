//! fusionsim — the hackathon tools on the fusion-sim engine, without noise.
//!
//! The same workflow as the organisers' toksym: build datasets, test-run a
//! submission, re-score a report, and serve the control-room UI — plus live
//! actuator control from a model.

mod mech;
mod model;
mod run;
mod scoring;
mod server;
mod teach;
mod testrun;

use std::path::PathBuf;
use std::time::Duration;

const HELP: &str = "fusionsim — симулятор токамака fusion-sim з інструментами хакатону (без шуму)

  fusionsim serve [--port 9999] [--models-dir DIR] [--python PY] [--out DIR] [--web DIR]
        Підняти інтерфейс: http://127.0.0.1:9999/

  fusionsim gen --shots N --seed S --out DIR [--stride N] [--extras program,transport,...]
        Зібрати датасет: measured.csv (100 каналів) і shots.csv.
        Групи extras: program transport pedestal impurity equilibrium profiles physics risk

  fusionsim check <тека здачі> [--python PY]
        Перевірити здачу: маніфест, файли, бібліотеки.

  fusionsim score --submission DIR [--shots 200] [--seed 1] [--lead 0.1] [--budget 0.05]
                  [--threshold X] [--report run.json] [--save-data DIR] [--python PY]
        Прогнати модель і видати звіт.

  fusionsim teach --shots N --seed S --out DIR [--mode teacher|label|student|none] [--student ТЕКА] [--alpha 0.5] [--horizon 1.0]
                  [--period 0.25] [--rollout-dt 0.02] [--stride N] [--extras risk,physics] [--jobs 8]
        Розряди під керуванням fixer (вчитель): measured.csv (+множники уставок, λ),
        decisions.csv (ризик і втрата кожної дії на кожному рішенні), shots.csv.
        --mode none — ті самі розряди без керування (точка відліку);
        --mode label — вчитель розмічає, але не діє (мітки для небезпечних станів);
        --mode student --student ТЕКА — керує модель-учень (+ аудит вчителем, --no-audit вимикає).

  fusionsim rescore <run.json> [--lead СЕК] [--budget ЧАСТКА] [--threshold X] [--out файл]
        Перерахувати звіт під інше правило.
";

struct Args(Vec<String>);

impl Args {
    fn get(&self, k: &str) -> Option<String> {
        self.0.iter().position(|a| a == k).and_then(|i| self.0.get(i + 1)).cloned()
    }
    fn num<T: std::str::FromStr>(&self, k: &str, d: T) -> T {
        self.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
    fn has(&self, k: &str) -> bool {
        self.0.iter().any(|a| a == k)
    }
    fn positional(&self, i: usize) -> Option<String> {
        self.0.iter().filter(|a| !a.starts_with("--")).nth(i).cloned()
    }
}

fn default_python() -> String {
    std::env::var("FUSIONSIM_PYTHON").unwrap_or_else(|_| "python3".into())
}

/// Resolve a path-like argument against the current directory; bare names
/// such as `python3` are left for PATH lookup.
fn absolute(p: String) -> String {
    if p.contains('/') {
        // Not canonicalize: that would resolve a venv's python symlink to the
        // system interpreter and lose the venv's packages.
        let path = PathBuf::from(&p);
        if path.is_absolute() {
            p
        } else {
            std::env::current_dir().map(|d| d.join(path).display().to_string()).unwrap_or(p)
        }
    } else {
        p
    }
}

fn main() {
    let all: Vec<String> = std::env::args().skip(1).collect();
    let cmd = all.first().cloned().unwrap_or_else(|| "serve".into());
    let a = Args(all.iter().skip(1).cloned().collect());
    // Models run from their own folder, so a relative interpreter path
    // (e.g. ../venv/bin/python) must be made absolute up front.
    let python = absolute(a.get("--python").unwrap_or_else(default_python));
    let result: Result<(), String> = match cmd.as_str() {
        "-h" | "--help" | "help" => {
            print!("{HELP}");
            Ok(())
        }
        "serve" | "ui" => {
            let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
            let web = a.get("--web").map(PathBuf::from).or_else(|| {
                let mut c = vec![PathBuf::from("web/dist")];
                if let Some(d) = &exe_dir {
                    c.push(d.join("../../web/dist"));
                }
                c.into_iter().find(|p| p.join("index.html").exists())
            });
            server::serve(server::Config {
                port: a.num("--port", 9999),
                python,
                models_dir: PathBuf::from(absolute(a.get("--models-dir").unwrap_or_else(|| "models".into()))),
                out_dir: a.get("--out").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("fusionsim-data")),
                web_dir: web,
                eval_shots: a.num("--eval-shots", 200),
                eval_seed: a.num("--eval-seed", 1),
            })
        }
        "gen" => {
            let out = PathBuf::from(a.get("--out").unwrap_or_else(|| "dataset".into()));
            let shots: usize = a.num("--shots", 20);
            let extras: Vec<String> = a.get("--extras").map(|s| s.split(',').map(String::from).collect()).unwrap_or_default();
            run::generate_dataset(&out, shots, a.num("--seed", 1), a.num("--stride", 1), &extras, |d| {
                if d % 10 == 0 || d == shots {
                    eprintln!("  {d}/{shots} розрядів");
                }
                true
            })
            .map(|r| eprintln!("{} рядків у {}/measured.csv\n{} розрядів у shots.csv, з них зірвалося {}", r.rows, r.dir, r.shots, r.disrupted))
            .map_err(|e| e.to_string())
        }
        "check" => {
            let dir = PathBuf::from(a.positional(0).unwrap_or_else(|| ".".into()));
            let ins = model::inspect(&dir, &python);
            if ins.problems.is_empty() {
                eprintln!("{}: готово до прогону ({}, файлів: {})", dir.display(), ins.entry.unwrap_or_default(), ins.files.len());
                Ok(())
            } else {
                for p in &ins.problems {
                    eprintln!("  ✗ {p}");
                }
                if let Some(s) = ins.suggestion {
                    eprintln!("Збережіть як toksym.json:\n{s}");
                }
                Err(format!("{} проблем", ins.problems.len()))
            }
        }
        "score" => (|| {
            let dir = PathBuf::from(a.get("--submission").ok_or("потрібен --submission ТЕКА")?);
            let ins = model::inspect(&dir, &python);
            if !ins.problems.is_empty() {
                return Err(ins.problems.join("; "));
            }
            let entry = ins.entry.unwrap();
            let shots: usize = a.num("--shots", 200);
            let save = a.get("--save-data").map(PathBuf::from);
            let cfg = testrun::TestRunConfig {
                model_dir: &dir, entry: &entry, python: &python,
                model_name: ins.manifest.and_then(|m| m.name), shots, seed: a.num("--seed", 1),
                rule: scoring::Rule {
                    lead_time_s: a.num("--lead", 0.1), fpr_budget: a.num("--budget", 0.05),
                    exclude_spontaneous: true, exclude_wall_contact: true,
                    threshold: a.get("--threshold").and_then(|v| v.parse().ok()),
                },
                step_timeout: Duration::from_secs_f64(a.num("--step-timeout", 1.0)),
                save_dir: save.as_deref(),
            };
            let rep = testrun::test_run(&cfg, |d| {
                if d % 10 == 0 || d == shots {
                    eprintln!("  {d}/{shots} розрядів");
                }
                true
            })?;
            if let Some(f) = a.get("--report") {
                std::fs::write(&f, serde_json::to_string_pretty(&rep).unwrap()).map_err(|e| e.to_string())?;
                eprintln!("Звіт: {f}\n");
            }
            testrun::print_summary(&rep);
            Ok(())
        })(),
        "teach" => (|| {
            let out = PathBuf::from(a.get("--out").unwrap_or_else(|| "teach".into()));
            let shots: usize = a.num("--shots", 20);
            let mode = teach::Mode::parse(&a.get("--mode").unwrap_or_else(|| "teacher".into()))?;
            let student = match a.get("--student") {
                Some(d) => {
                    let dir = PathBuf::from(absolute(d));
                    let ins = model::inspect(&dir, &python);
                    if !ins.problems.is_empty() {
                        return Err(ins.problems.join("; "));
                    }
                    Some(teach::StudentSpec { dir, entry: ins.entry.unwrap(), python: python.clone() })
                }
                None if mode == teach::Mode::Student => return Err("--mode student потребує --student ТЕКА".into()),
                None => None,
            };
            let cfg = teach::TeachConfig {
                shots,
                seed: a.num("--seed", 1),
                mode,
                fixer: tok_sym_core::fixer::FixerConfig {
                    horizon: a.num("--horizon", 1.0),
                    dt: a.num("--rollout-dt", 0.02),
                    alpha: a.num("--alpha", 0.5),
                },
                period: a.num("--period", 0.25),
                stride: a.num("--stride", 1),
                extras: a.get("--extras").map(|s| s.split(',').map(String::from).collect()).unwrap_or_default(),
                jobs: a.num("--jobs", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)),
                student,
                audit: !a.has("--no-audit"),
            };
            let msg = teach::teach(&out, &cfg, |d| eprintln!("  {d}/{shots} розрядів"))?;
            eprintln!("{msg}\nФайли: {}/measured.csv, decisions.csv, shots.csv", out.display());
            Ok(())
        })(),
        "rescore" => (|| {
            let f = a.positional(0).ok_or("потрібен файл звіту")?;
            let mut rep: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&f).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            scoring::rescore(&mut rep, a.get("--lead").and_then(|v| v.parse().ok()), a.get("--budget").and_then(|v| v.parse().ok()), a.get("--threshold").and_then(|v| v.parse().ok()))?;
            if let Some(o) = a.get("--out") {
                std::fs::write(o, serde_json::to_string_pretty(&rep).unwrap()).map_err(|e| e.to_string())?;
            }
            testrun::print_summary(&rep);
            Ok(())
        })(),
        other => Err(format!("невідома команда {other}; див. fusionsim --help")),
    };
    if let Err(e) = result {
        eprintln!("помилка: {e}");
        std::process::exit(1);
    }
    let _ = a.has("--no-open");
}
