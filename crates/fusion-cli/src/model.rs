//! A submitted model: manifest checks and the line protocol toksym speaks.
//!
//! ```text
//! → {"type":"hello","protocol":1,"dt":0.005,"n_shots":200}   ← {"type":"ready"}
//! → {"type":"shot_start","shot_id":0,"dt":..,"duration":..,"scenario":..,"channels":[...]}
//! → {"type":"step","t":0.005,"v":[...]}                       ← 0.0123
//! → {"type":"shot_end","shot_id":0}
//! → {"type":"bye"}
//! ```
//! Lines starting with `#` are the model's log, not answers.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tok_sym_core::channels::CHANNEL_NAMES;

/// The Python wrapper every submission imports (`from toksym_model import serve`).
pub const WRAPPER: &str = include_str!("../toksym_model.py");

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    pub schema: Option<u32>,
    pub entry: Option<String>,
    pub files: Option<Vec<String>>,
    #[serde(default)]
    pub requires: Vec<String>,
    pub name: Option<String>,
    pub notes: Option<String>,
}

/// What a submission folder looks like, and everything wrong with it.
#[derive(Debug, Clone, Serialize)]
pub struct Inspection {
    pub dir: String,
    pub manifest: Option<Manifest>,
    pub entry: Option<String>,
    pub files: Vec<String>,
    pub problems: Vec<String>,
    pub suggestion: Option<String>,
}

fn list_files(dir: &Path, base: &Path, out: &mut Vec<String>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "__pycache__" {
                continue;
            }
            if p.is_dir() {
                list_files(&p, base, out);
            } else if let Ok(rel) = p.strip_prefix(base) {
                out.push(rel.display().to_string());
            }
        }
    }
}

/// Check a submission folder the way `toksym check` does: report every
/// problem at once.
pub fn inspect(dir: &Path, python: &str) -> Inspection {
    let mut files = Vec::new();
    list_files(dir, dir, &mut files);
    files.sort();
    let mut problems = Vec::new();
    let mpath = dir.join("toksym.json");
    let mut suggestion = None;
    let manifest: Option<Manifest> = if !mpath.exists() {
        problems.push("немає toksym.json — без нього невідомо, що запускати і які файли мають бути на місці".into());
        let py: Vec<&String> = files.iter().filter(|f| f.ends_with(".py") && !f.ends_with("toksym_model.py")).collect();
        let entry = py.iter().find(|f| f.as_str() == "model.py").or(py.first()).map(|s| s.to_string());
        let others: Vec<&String> = files.iter().filter(|f| Some(f.to_string()) != entry && !f.ends_with("toksym_model.py")).collect();
        suggestion = Some(serde_json::to_string_pretty(&json!({
            "schema": 1, "entry": entry.unwrap_or_else(|| "model.py".into()), "files": others, "requires": ["numpy"]
        })).unwrap());
        None
    } else {
        match std::fs::read_to_string(&mpath).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str::<Manifest>(&s).map_err(|e| e.to_string())) {
            Ok(m) => Some(m),
            Err(e) => {
                problems.push(format!("toksym.json не розбирається: {e}"));
                None
            }
        }
    };
    let mut entry = None;
    if let Some(m) = &manifest {
        if m.schema.is_none() {
            problems.push("у toksym.json немає поля schema".into());
        }
        match &m.entry {
            None => problems.push("у toksym.json немає поля entry".into()),
            Some(e) if !e.ends_with(".py") => problems.push(format!("entry «{e}» — не файл .py")),
            Some(e) if !dir.join(e).is_file() => problems.push(format!("entry «{e}» не знайдено в теці")),
            Some(e) => entry = Some(e.clone()),
        }
        match &m.files {
            None => problems.push("у toksym.json немає поля files (порожній список теж годиться)".into()),
            Some(fs) => {
                for f in fs {
                    let p = dir.join(f);
                    match std::fs::metadata(&p) {
                        Err(_) => problems.push(format!("файлу «{f}» зі списку files немає")),
                        Ok(md) if md.len() == 0 => problems.push(format!("файл «{f}» порожній")),
                        _ => {}
                    }
                }
            }
        }
        for lib in &m.requires {
            let module = lib.replace('-', "_").replace("scikit_learn", "sklearn");
            let ok = Command::new(python).args(["-c", &format!("import {module}")])
                .stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
            if !ok {
                problems.push(format!("бібліотеки «{lib}» не встановлено для {python}"));
            }
        }
    }
    Inspection { dir: dir.display().to_string(), manifest, entry, files, problems, suggestion }
}

/// What a model said on one step.
#[derive(Debug, Clone)]
pub struct Answer {
    pub p: f64,
    /// Mechanism shares etc., when the model sends a JSON object.
    pub info: Option<serde_json::Value>,
}

fn parse_answer(line: &str) -> Result<Answer, String> {
    let l = line.trim();
    let (p, info) = if l.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(l).map_err(|e| format!("модель відповіла не JSON: {e}: «{l}»"))?;
        let p = v["p"].as_f64().ok_or_else(|| format!("у відповіді моделі немає числа p: «{l}»"))?;
        (p, Some(v))
    } else {
        (l.parse::<f64>().map_err(|_| format!("модель відповіла не числом: «{l}»"))?, None)
    };
    Ok(Answer { p: if p.is_nan() { 0.0 } else { p.clamp(0.0, 1.0) }, info })
}

/// A running model process.
pub struct ModelProcess {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    pub log: Vec<String>,
    pub label: String,
    _wrapper_dir: PathBuf,
}

impl ModelProcess {
    /// Start `python entry` inside `dir` with the wrapper importable.
    pub fn start(dir: &Path, entry: &str, python: &str, n_shots: usize) -> Result<Self, String> {
        let wrapper_dir = std::env::temp_dir().join(format!("fusionsim-wrapper-{}", std::process::id()));
        std::fs::create_dir_all(&wrapper_dir).map_err(|e| e.to_string())?;
        std::fs::write(wrapper_dir.join("toksym_model.py"), WRAPPER).map_err(|e| e.to_string())?;
        let mut pythonpath = wrapper_dir.display().to_string();
        if let Ok(old) = std::env::var("PYTHONPATH") {
            pythonpath = format!("{pythonpath}:{old}");
        }
        let mut child = Command::new(python)
            .arg("-u")
            .arg(entry)
            .current_dir(dir)
            .env("PYTHONPATH", pythonpath)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("не вдалося запустити {python} {entry}: {e}"))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if tx2.send(format!("#stderr {line}")).is_err() {
                    break;
                }
            }
        });
        let mut m = ModelProcess {
            child, stdin, lines: rx, log: Vec::new(),
            label: format!("{python} {}", dir.join(entry).display()), _wrapper_dir: wrapper_dir,
        };
        m.send(&json!({"type": "hello", "protocol": 1, "dt": 0.005, "n_shots": n_shots, "rich": true}).to_string())?;
        let reply = m.answer(Duration::from_secs(30)).map_err(|e| format!("модель не сказала ready за 30 с: {e}"))?;
        if !reply.contains("ready") {
            return Err(format!("замість ready модель відповіла: {reply}"));
        }
        Ok(m)
    }

    fn send(&mut self, line: &str) -> Result<(), String> {
        writeln!(self.stdin, "{line}").and_then(|_| self.stdin.flush()).map_err(|e| format!("модель закрила вхід: {e}{}", self.tail()))
    }

    fn tail(&self) -> String {
        let t: Vec<&String> = self.log.iter().rev().take(15).collect();
        if t.is_empty() {
            String::new()
        } else {
            format!("\n{}", t.into_iter().rev().cloned().collect::<Vec<_>>().join("\n"))
        }
    }

    /// Next non-log line within `timeout`.
    fn answer(&mut self, timeout: Duration) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(l) if l.starts_with('#') => {
                    self.log.push(l);
                    if self.log.len() > 500 {
                        self.log.drain(..250);
                    }
                }
                Ok(l) => return Ok(l),
                Err(RecvTimeoutError::Timeout) => return Err(format!("мовчання понад {:.1} с{}", timeout.as_secs_f64(), self.tail())),
                Err(RecvTimeoutError::Disconnected) => {
                    std::thread::sleep(Duration::from_millis(100));
                    while let Ok(l) = self.lines.try_recv() {
                        self.log.push(l);
                    }
                    return Err(format!("модель впала{}", self.tail()));
                }
            }
        }
    }

    pub fn shot_start(&mut self, shot_id: u64, duration: f64, scenario: &str) -> Result<(), String> {
        self.send(&json!({"type": "shot_start", "shot_id": shot_id, "dt": 0.005, "duration": duration,
            "scenario": scenario, "shift": false, "salted": false, "channels": CHANNEL_NAMES.to_vec()}).to_string())
    }

    /// One step: returns the model's answer (p clipped to [0, 1], NaN → 0) and
    /// the latency. Besides a bare number (toksym protocol 1) a model may answer
    /// with a JSON object `{"p": .., "share_pct": {..}, "risk_total": ..}`; the
    /// extra fields travel along as `info`.
    pub fn step(&mut self, t: f64, v: &[f64], timeout: Duration) -> Result<(Answer, Duration), String> {
        let vals: Vec<serde_json::Value> = v.iter().map(|x| if x.is_finite() { json!(x) } else { serde_json::Value::Null }).collect();
        let t0 = Instant::now();
        self.send(&json!({"type": "step", "t": (t * 1e4).round() / 1e4, "v": vals}).to_string())?;
        let line = self.answer(timeout)?;
        let dt = t0.elapsed();
        Ok((parse_answer(&line)?, dt))
    }

    /// A step with extra context for the model (fusionsim only): e.g. the
    /// fixer's current programme multipliers. The wrapper exposes it as
    /// `model.extra` before calling `predict`.
    pub fn step_extra(&mut self, t: f64, v: &[f64], extra: serde_json::Value, timeout: Duration) -> Result<(Answer, Duration), String> {
        let vals: Vec<serde_json::Value> = v.iter().map(|x| if x.is_finite() { json!(x) } else { serde_json::Value::Null }).collect();
        let t0 = Instant::now();
        self.send(&json!({"type": "step", "t": (t * 1e4).round() / 1e4, "v": vals, "extra": extra}).to_string())?;
        let line = self.answer(timeout)?;
        let dt = t0.elapsed();
        Ok((parse_answer(&line)?, dt))
    }

    pub fn shot_end(&mut self, shot_id: u64) -> Result<(), String> {
        self.send(&json!({"type": "shot_end", "shot_id": shot_id}).to_string())
    }
}

impl Drop for ModelProcess {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "{}", json!({"type": "bye"}));
        let _ = self.stdin.flush();
        std::thread::sleep(Duration::from_millis(50));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
