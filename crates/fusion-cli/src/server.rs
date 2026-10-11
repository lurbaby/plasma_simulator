//! Local HTTP server: the web UI plus a toksym-style API.
//!
//! toksym-compatible: `/api/info`, `/api/dataset` (+`/progress`, `/file`),
//! `/api/validate` (+`/progress`, `/file`), `/api/model/{reset,file,list}`.
//! Ours: `/api/models` (pick a local submission instead of uploading),
//! `/api/rescore`, and `/api/live/{start,step,stop}` — a model answering in
//! real time for the interactive control room and its auto-controller.


use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::model::{inspect, ModelProcess};
use crate::run::{generate_dataset, EXTRA_GROUPS};
use crate::scoring::{rescore, Rule};
use crate::testrun::{test_run, TestRunConfig};

pub struct Config {
    pub port: u16,
    pub python: String,
    pub models_dir: PathBuf,
    pub out_dir: PathBuf,
    pub web_dir: Option<PathBuf>,
    pub eval_shots: usize,
    pub eval_seed: u64,
}

#[derive(Default)]
struct Job {
    running: bool,
    done: usize,
    total: usize,
    error: Option<String>,
    result: Option<Value>,
}

struct Live {
    model: ModelProcess,
}

struct State {
    cfg: Config,
    upload_dir: PathBuf,
    dataset: Arc<Mutex<Job>>,
    dataset_cancel: Arc<AtomicBool>,
    validate: Arc<Mutex<Job>>,
    validate_cancel: Arc<AtomicBool>,
    live: Mutex<Option<Live>>,
}

type Resp = Response<std::io::Cursor<Vec<u8>>>;

fn json_resp(code: u16, v: &Value) -> Resp {
    Response::from_string(v.to_string())
        .with_status_code(code)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

fn err(code: u16, msg: impl Into<String>) -> Resp {
    json_resp(code, &json!({"error": msg.into()}))
}

fn query(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(x) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(x);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn body_json(req: &mut Request) -> Value {
    let mut s = String::new();
    let _ = req.as_reader().read_to_string(&mut s);
    serde_json::from_str(&s).unwrap_or(Value::Null)
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript",
        "css" => "text/css",
        "wasm" => "application/wasm",
        "json" | "webmanifest" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "csv" => "text/csv; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn file_resp(path: &Path, download: Option<&str>) -> Resp {
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut r = Response::from_data(bytes).with_header(Header::from_bytes("Content-Type", content_type(path)).unwrap());
            if let Some(name) = download {
                r = r.with_header(Header::from_bytes("Content-Disposition", format!("attachment; filename=\"{name}\"")).unwrap());
            }
            r
        }
        Err(_) => err(404, format!("{} не знайдено", path.display())),
    }
}

/// Local submissions: every folder under `models_dir` with a toksym.json or a .py.
fn list_models(st: &State) -> Value {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&st.cfg.models_dir) {
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            let has_py = std::fs::read_dir(&d).map(|r| r.flatten().any(|e| e.path().extension().is_some_and(|x| x == "py"))).unwrap_or(false);
            if !has_py && !d.join("toksym.json").exists() {
                continue;
            }
            let ins = inspect(&d, &st.cfg.python);
            out.push(json!({
                "id": d.display().to_string(),
                "folder": d.file_name().map(|n| n.to_string_lossy().to_string()),
                "name": ins.manifest.as_ref().and_then(|m| m.name.clone()),
                "notes": ins.manifest.as_ref().and_then(|m| m.notes.clone()),
                "entry": ins.entry, "files": ins.files.len(), "problems": ins.problems,
            }));
        }
    }
    json!({"dir": st.cfg.models_dir.display().to_string(), "python": st.cfg.python, "models": out})
}

/// Resolve the model a request names: "uploaded" or a local folder path.
fn resolve_model(st: &State, id: &str) -> Result<(PathBuf, String, Option<String>), String> {
    let dir = if id == "uploaded" || id.is_empty() { upload_root(&st.upload_dir) } else { PathBuf::from(id) };
    let ins = inspect(&dir, &st.cfg.python);
    if !ins.problems.is_empty() {
        return Err(ins.problems.join("; "));
    }
    let entry = ins.entry.ok_or("немає точки входу")?;
    let name = ins.manifest.and_then(|m| m.name).or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()));
    Ok((dir, entry, name))
}

/// A dropped folder arrives as `myfolder/model.py`; the manifest may sit one level down.
fn upload_root(dir: &Path) -> PathBuf {
    if dir.join("toksym.json").exists() {
        return dir.to_path_buf();
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().join("toksym.json").exists() {
                return e.path();
            }
        }
    }
    dir.to_path_buf()
}

fn job_json(j: &Job) -> Value {
    json!({"running": j.running, "done": j.done, "total": j.total, "error": j.error, "result": j.result})
}

fn handle(st: &Arc<State>, mut req: Request) {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("/").to_string();
    let method = req.method().clone();
    let resp: Resp = match (method, path.as_str()) {
        (Method::Get, "/api/info") => json_resp(200, &json!({
            "dt": 0.005, "eval_shots": st.cfg.eval_shots, "eval_seed": st.cfg.eval_seed, "python": st.cfg.python,
            "version": env!("CARGO_PKG_VERSION"), "engine": "fusion-sim", "noise": false,
            "models_dir": st.cfg.models_dir.display().to_string(), "extras": EXTRA_GROUPS,
        })),
        (Method::Get, "/api/models") => json_resp(200, &list_models(st)),
        (Method::Post, "/api/model/reset") => {
            let _ = std::fs::remove_dir_all(&st.upload_dir);
            let _ = std::fs::create_dir_all(&st.upload_dir);
            json_resp(200, &json!({"ok": true}))
        }
        (Method::Post, "/api/model/file") => match query(&url, "path") {
            Some(rel) if !rel.split('/').any(|c| c == ".." || c.is_empty()) => {
                let dest = st.upload_dir.join(&rel);
                let mut buf = Vec::new();
                let _ = req.as_reader().read_to_end(&mut buf);
                match dest.parent().map(std::fs::create_dir_all).transpose().and_then(|_| std::fs::write(&dest, buf)) {
                    Ok(_) => json_resp(200, &json!({"ok": true})),
                    Err(e) => err(500, e.to_string()),
                }
            }
            _ => err(400, "поганий шлях файлу"),
        },
        (Method::Get, "/api/model/list") => {
            let root = upload_root(&st.upload_dir);
            let ins = inspect(&root, &st.cfg.python);
            let rel_root = root.strip_prefix(&st.upload_dir).ok().map(|p| p.display().to_string()).filter(|s| !s.is_empty());
            json_resp(200, &json!({"dir": root.display().to_string(), "root": rel_root, "manifest": ins.manifest,
                "entry": ins.entry, "files": ins.files, "problems": ins.problems, "suggestion": ins.suggestion, "python": st.cfg.python}))
        }
        (Method::Post, "/api/dataset") => {
            let b = body_json(&mut req);
            let mut job = st.dataset.lock().unwrap();
            if job.running {
                err(409, "датасет уже збирається")
            } else {
                let shots = b["shots"].as_u64().unwrap_or(20).clamp(1, 5000) as usize;
                let seed = b["seed"].as_u64().unwrap_or(1);
                let stride = b["stride"].as_u64().unwrap_or(1).max(1) as usize;
                let extras: Vec<String> = match &b["extras"] {
                    Value::String(s) => s.split(',').filter(|x| !x.is_empty()).map(String::from).collect(),
                    Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
                    _ => vec![],
                };
                *job = Job { running: true, total: shots, ..Default::default() };
                drop(job);
                let st2 = st.clone();
                st.dataset_cancel.store(false, Ordering::SeqCst);
                std::thread::spawn(move || {
                    let out = st2.cfg.out_dir.join("dataset");
                    let jobref = st2.dataset.clone();
                    let cancel = st2.dataset_cancel.clone();
                    let r = generate_dataset(&out, shots, seed, stride, &extras, |done| {
                        jobref.lock().unwrap().done = done;
                        !cancel.load(Ordering::SeqCst)
                    });
                    let mut j = st2.dataset.lock().unwrap();
                    j.running = false;
                    match r {
                        Ok(res) => j.result = Some(serde_json::to_value(res).unwrap()),
                        Err(e) => j.error = Some(e.to_string()),
                    }
                });
                json_resp(200, &json!({"ok": true}))
            }
        }
        (Method::Post, "/api/dataset/cancel") => {
            st.dataset_cancel.store(true, Ordering::SeqCst);
            json_resp(200, &json!({"ok": true}))
        }
        (Method::Get, "/api/dataset/progress") => json_resp(200, &job_json(&st.dataset.lock().unwrap())),
        (Method::Get, "/api/dataset/file") => match query(&url, "name").as_deref() {
            Some(n @ ("measured.csv" | "shots.csv")) => file_resp(&st.cfg.out_dir.join("dataset").join(n), Some(n)),
            _ => err(404, "немає такого файлу"),
        },
        (Method::Post, "/api/validate") => {
            let b = body_json(&mut req);
            let mut job = st.validate.lock().unwrap();
            if job.running {
                err(409, "прогін уже йде")
            } else {
                let model_id = b["model"].as_str().unwrap_or("uploaded").to_string();
                match resolve_model(st, &model_id) {
                    Err(e) => err(400, e),
                    Ok((dir, entry, name)) => {
                        let shots = b["shots"].as_u64().map(|x| x as usize).unwrap_or(st.cfg.eval_shots).clamp(1, 2000);
                        let seed = b["seed"].as_u64().unwrap_or(st.cfg.eval_seed);
                        let rule = Rule {
                            lead_time_s: b["lead_time_s"].as_f64().unwrap_or(0.1),
                            fpr_budget: b["fpr_budget"].as_f64().unwrap_or(0.05),
                            exclude_spontaneous: true,
                            exclude_wall_contact: true,
                            threshold: b["threshold"].as_f64(),
                        };
                        *job = Job { running: true, total: shots, ..Default::default() };
                        drop(job);
                        st.validate_cancel.store(false, Ordering::SeqCst);
                        let st2 = st.clone();
                        std::thread::spawn(move || {
                            let save = st2.cfg.out_dir.join("validate");
                            let jobref = st2.validate.clone();
                            let cancel = st2.validate_cancel.clone();
                            let cfg = TestRunConfig {
                                model_dir: &dir, entry: &entry, python: &st2.cfg.python, model_name: name,
                                shots, seed, rule, step_timeout: Duration::from_secs(1), save_dir: Some(&save),
                            };
                            let r = test_run(&cfg, |done| {
                                jobref.lock().unwrap().done = done;
                                !cancel.load(Ordering::SeqCst)
                            });
                            let mut j = st2.validate.lock().unwrap();
                            j.running = false;
                            match r {
                                Ok(rep) => j.result = Some(rep),
                                Err(e) => j.error = Some(e),
                            }
                        });
                        json_resp(200, &json!({"ok": true}))
                    }
                }
            }
        }
        (Method::Post, "/api/validate/cancel") => {
            st.validate_cancel.store(true, Ordering::SeqCst);
            json_resp(200, &json!({"ok": true}))
        }
        (Method::Get, "/api/validate/progress") => json_resp(200, &job_json(&st.validate.lock().unwrap())),
        (Method::Get, "/api/validate/file") => match query(&url, "name").as_deref() {
            Some(n @ ("measured.csv" | "shots.csv" | "run.json")) => file_resp(&st.cfg.out_dir.join("validate").join(n), Some(n)),
            _ => err(404, "немає такого файлу"),
        },
        (Method::Post, "/api/rescore") => {
            let mut b = body_json(&mut req);
            let (lead, budget, thr) = (b["lead_time_s"].as_f64(), b["fpr_budget"].as_f64(), b["threshold"].as_f64());
            let mut rep = b["report"].take();
            match rescore(&mut rep, lead, budget, thr) {
                Ok(()) => json_resp(200, &rep),
                Err(e) => err(400, e),
            }
        }
        (Method::Post, "/api/live/start") => {
            let b = body_json(&mut req);
            let id = b["model"].as_str().unwrap_or("uploaded").to_string();
            let mut live = st.live.lock().unwrap();
            *live = None;
            match resolve_model(st, &id).and_then(|(dir, entry, name)| {
                let mut m = ModelProcess::start(&dir, &entry, &st.cfg.python, 1)?;
                m.shot_start(0, b["duration"].as_f64().unwrap_or(20.0), b["scenario"].as_str().unwrap_or("interactive"))?;
                Ok((m, name))
            }) {
                Ok((m, name)) => {
                    *live = Some(Live { model: m });
                    json_resp(200, &json!({"ok": true, "model": name}))
                }
                Err(e) => err(400, e),
            }
        }
        (Method::Post, "/api/live/step") => {
            let b = body_json(&mut req);
            let mut live = st.live.lock().unwrap();
            match live.as_mut() {
                None => err(409, "модель не запущена"),
                Some(l) => {
                    // A batch of steps: {"steps": [{"t":..,"v":[..]}, ...]} → {"p": [..]}
                    let steps: Vec<Value> = match &b["steps"] {
                        Value::Array(a) => a.clone(),
                        _ => vec![b.clone()],
                    };
                    let mut ps = Vec::new();
                    let mut infos = Vec::new();
                    let mut failure = None;
                    for s in steps {
                        let v: Vec<f64> = s["v"].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect()).unwrap_or_default();
                        let t = s["t"].as_f64().unwrap_or(0.0);
                        let r = match s.get("extra") {
                            Some(x) if !x.is_null() => l.model.step_extra(t, &v, x.clone(), Duration::from_secs(1)),
                            _ => l.model.step(t, &v, Duration::from_secs(1)),
                        };
                        match r {
                            Ok((ans, _)) => {
                                ps.push(ans.p);
                                infos.push(ans.info.unwrap_or(Value::Null));
                            }
                            Err(e) => {
                                failure = Some(e);
                                break;
                            }
                        }
                    }
                    match failure {
                        Some(e) => {
                            *live = None;
                            err(500, e)
                        }
                        None => json_resp(200, &json!({"p": ps, "info": infos})),
                    }
                }
            }
        }
        (Method::Post, "/api/live/stop") => {
            *st.live.lock().unwrap() = None;
            json_resp(200, &json!({"ok": true}))
        }
        (Method::Get, p) if !p.starts_with("/api/") => match &st.cfg.web_dir {
            Some(web) => {
                let rel = p.trim_start_matches('/');
                let f = web.join(if rel.is_empty() { "index.html" } else { rel });
                if f.is_file() && !rel.contains("..") {
                    file_resp(&f, None)
                } else if Path::new(rel).extension().is_some() {
                    err(404, format!("{rel} не знайдено"))
                } else {
                    file_resp(&web.join("index.html"), None)
                }
            }
            None => err(404, "інтерфейс не зібрано: npm run build у web/"),
        },
        _ => err(404, format!("немає {path}")),
    };
    let _ = req.respond(resp);
}

pub fn serve(cfg: Config) -> Result<(), String> {
    let addr = format!("127.0.0.1:{}", cfg.port);
    let server = Server::http(&addr).map_err(|e| format!("{addr}: {e}"))?;
    let upload_dir = std::env::temp_dir().join(format!("fusionsim-model-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&upload_dir);
    let _ = std::fs::create_dir_all(&cfg.out_dir);
    eprintln!("Інтерфейс: http://{addr}/");
    eprintln!("Моделі: {}   Python: {}   Дані: {}", cfg.models_dir.display(), cfg.python, cfg.out_dir.display());
    let st = Arc::new(State {
        cfg, upload_dir,
        dataset: Arc::new(Mutex::new(Job::default())), dataset_cancel: Arc::new(AtomicBool::new(false)),
        validate: Arc::new(Mutex::new(Job::default())), validate_cancel: Arc::new(AtomicBool::new(false)),
        live: Mutex::new(None),
    });
    for req in server.incoming_requests() {
        let st = st.clone();
        std::thread::spawn(move || handle(&st, req));
    }
    Ok(())
}
