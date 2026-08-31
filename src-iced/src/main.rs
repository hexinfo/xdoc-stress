//! x-doc 核心链路压测 —— iced GUI（tiny-skia 软件渲染，纯 Rust，无 GPU/GTK，目标 glibc 2.28）

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod formatters;
mod model;
mod palette;
mod styles;
mod view;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iced::{Size, Subscription, Task, window};
use stress_core::engine;
use stress_core::types::{RunHandle, RunSummary, StressConfig};

use model::{FileEntry, ResultRowUi};

const CSV_HEADER: &str = "序号,文件,页数,上传,就绪,下载,转换,首页,全页,端到端,状态,错误";
const COPY_HEADER: &str = "#\t文件\t页数\t上传\t就绪\t下载\t转换\t首页\t全页\t端到端\t状态";

/// 轮询快照（后台读取 RunStatus 后一次性带回 UI）
#[derive(Clone)]
pub struct Snapshot {
    running: bool,
    rows: Vec<(ResultRowUi, String)>,
    log: Option<String>,
    log_len: usize,
    success: usize,
    total: usize,
    summary: Option<RunSummary>,
}

#[derive(Clone)]
pub enum Message {
    PickFolder,
    PickFiles,
    ClearFiles,
    ToggleAll(bool),
    ToggleFile(usize, bool),
    BaseUrlChanged(String),
    AuthTokenChanged(String),
    AuthHeadersChanged(String),
    ConcurrencyChanged(String),
    RepeatsChanged(String),
    RangeChunkChanged(String),
    TileBatchChanged(String),
    PollIntervalChanged(String),
    PollMaxChanged(String),
    CopyResults,
    ExportCsv,
    StartTest,
    StopTest,
    PollTick,
    Polled(Box<Snapshot>),
    ResultRowHover(Option<usize>),
    FileRowHover(Option<usize>),
    /// 点击文件列表行(Shift+点击为范围多选)
    RowClick(usize),
    /// 键盘修饰键变化(跟踪 Shift)
    Modifiers(iced::keyboard::Modifiers),
}

/// 右栏配置（全部保持字符串，开始时再解析，非法值回落默认）
#[derive(Clone)]
pub struct Config {
    pub base_url: String,
    pub auth_token: String,
    pub auth_headers: String,
    pub concurrency: String,
    pub repeats: String,
    pub range_chunk: String,
    pub tile_batch: String,
    pub poll_interval: String,
    pub poll_max: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8080/dvs".into(),
            auth_token: String::new(),
            auth_headers: String::new(),
            concurrency: "1".into(),
            repeats: "2".into(),
            range_chunk: "1048576".into(),
            tile_batch: "10".into(),
            poll_interval: "1000".into(),
            poll_max: "600".into(),
        }
    }
}

pub struct App {
    pub files: Vec<FileEntry>,
    pub all_checked: bool,
    pub cfg: Config,
    pub running: bool,
    /// 压测线程存活标志（线程结束时落回 false）
    pub run_flag: Arc<AtomicBool>,
    pub handle: Arc<RunHandle>,
    pub rows: Vec<(ResultRowUi, String)>,
    pub status_text: String,
    pub log_text: String,
    pub last_log_len: usize,
    pub hovered_result_row: Option<usize>,
    pub hovered_file_row: Option<usize>,
    pub modifiers: iced::keyboard::Modifiers,
    /// 文件列表范围多选的锚点行
    pub click_anchor: Option<usize>,
}

impl App {
    fn new() -> (App, Task<Message>) {
        let app = Self {
            files: Vec::new(),
            all_checked: false,
            cfg: Config::default(),
            running: false,
            run_flag: Arc::new(AtomicBool::new(false)),
            handle: Arc::new(RunHandle::new()),
            rows: Vec::new(),
            status_text: "就绪".into(),
            log_text: String::new(),
            last_log_len: 0,
            hovered_result_row: None,
            hovered_file_row: None,
            modifiers: iced::keyboard::Modifiers::default(),
            click_anchor: None,
        };
        (app, Task::none())
    }

    fn sync_all_checked(&mut self) {
        self.all_checked = !self.files.is_empty() && self.files.iter().all(|f| f.checked);
    }

    /// 去重添加文件（默认勾选），全为重复时提示
    fn add_files(&mut self, paths: Vec<std::path::PathBuf>) {
        let mut added = 0;
        for p in paths {
            let path = p.to_string_lossy().to_string();
            if self.files.iter().any(|f| f.path == path) {
                continue;
            }
            let name = p.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            self.files.push(FileEntry { name, path, size: formatters::fmt_size(size), checked: true });
            added += 1;
        }
        if added > 0 {
            self.sync_all_checked();
        } else {
            self.status_text = "所选文件已全部添加".into();
        }
    }

    /// 校验配置并发起压测（后台线程内建 tokio runtime 跑引擎）
    fn start_test(&mut self) {
        if self.running {
            return;
        }
        let files: Vec<String> = self.files.iter().filter(|f| f.checked).map(|f| f.path.clone()).collect();
        if files.is_empty() {
            self.status_text = "请先添加并勾选文件".into();
            return;
        }
        let base_url = self.cfg.base_url.trim().to_string();
        if base_url.is_empty() {
            self.status_text = "请填 BASE_URL".into();
            return;
        }
        let headers_raw = self.cfg.auth_headers.trim().to_string();
        let auth_headers = if headers_raw.is_empty() {
            serde_json::json!({})
        } else {
            match serde_json::from_str(&headers_raw) {
                Ok(v) => v,
                Err(_) => {
                    self.status_text = "AUTH_HEADERS 不是合法 JSON".into();
                    return;
                }
            }
        };

        let parse = |s: &str, d: &str| {
            s.trim().parse::<f64>().map(|v| v.max(1.0)).unwrap_or_else(|_| d.parse().unwrap_or(1.0))
        };
        let cfg = StressConfig {
            base_url,
            auth_token: self.cfg.auth_token.trim().to_string(),
            auth_headers,
            steps: vec!["upload".into(), "convert".into(), "range".into(), "excel".into()],
            concurrency: parse(&self.cfg.concurrency, "1") as u32,
            repeats: parse(&self.cfg.repeats, "2") as u32,
            range_chunk: parse(&self.cfg.range_chunk, "1048576") as u64,
            tile_batch: parse(&self.cfg.tile_batch, "10") as usize,
            poll_interval_ms: parse(&self.cfg.poll_interval, "1000").max(100.0) as u64,
            poll_max_times: parse(&self.cfg.poll_max, "600") as u32,
            files,
        };

        self.running = true;
        self.run_flag.store(true, Ordering::Relaxed);
        self.status_text = format!("运行中… 0/{}", cfg.files.len());
        self.log_text.clear();
        self.last_log_len = 0;

        // 复位停止开关：引擎读 handle.stop_flag，停止按钮才能真正中断本轮
        let stop = self.handle.stop_flag.clone();
        stop.store(false, Ordering::Relaxed);
        let status = self.handle.status.clone();
        let run_flag = self.run_flag.clone();
        let file_list: Vec<(String, String)> = cfg
            .files
            .iter()
            .map(|p| {
                let name = std::path::Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.clone());
                (name, p.clone())
            })
            .collect();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                engine::start_stress(cfg, stop, status, file_list).await;
            });
            run_flag.store(false, Ordering::Relaxed);
        });
    }
}

fn update(app: &mut App, msg: Message) -> Task<Message> {
    match msg {
        Message::PickFolder => {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                let mut paths: Vec<_> = std::fs::read_dir(&dir)
                    .map(|entries| {
                        entries
                            .flatten()
                            .map(|e| e.path())
                            .filter(|p| {
                                p.is_file()
                                    && !p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                paths.sort();
                if paths.is_empty() {
                    app.status_text = "该文件夹没有可用文件".into();
                } else {
                    app.add_files(paths);
                }
            }
            Task::none()
        }
        Message::PickFiles => {
            if let Some(sel) = rfd::FileDialog::new().pick_files() {
                app.add_files(sel);
            }
            Task::none()
        }
        Message::ClearFiles => {
            app.files.clear();
            app.all_checked = false;
            app.status_text = "就绪".into();
            Task::none()
        }
        Message::ToggleAll(checked) => {
            for f in &mut app.files {
                f.checked = checked;
            }
            app.all_checked = checked;
            Task::none()
        }
        Message::ToggleFile(i, checked) => {
            if let Some(f) = app.files.get_mut(i) {
                f.checked = checked;
            }
            app.sync_all_checked();
            Task::none()
        }
        Message::BaseUrlChanged(s) => {
            app.cfg.base_url = s;
            Task::none()
        }
        Message::AuthTokenChanged(s) => {
            app.cfg.auth_token = s;
            Task::none()
        }
        Message::AuthHeadersChanged(s) => {
            app.cfg.auth_headers = s;
            Task::none()
        }
        Message::ConcurrencyChanged(s) => {
            app.cfg.concurrency = s;
            Task::none()
        }
        Message::RepeatsChanged(s) => {
            app.cfg.repeats = s;
            Task::none()
        }
        Message::RangeChunkChanged(s) => {
            app.cfg.range_chunk = s;
            Task::none()
        }
        Message::TileBatchChanged(s) => {
            app.cfg.tile_batch = s;
            Task::none()
        }
        Message::PollIntervalChanged(s) => {
            app.cfg.poll_interval = s;
            Task::none()
        }
        Message::PollMaxChanged(s) => {
            app.cfg.poll_max = s;
            Task::none()
        }
        Message::CopyResults => {
            if app.rows.is_empty() {
                app.status_text = "暂无结果可复制".into();
            } else {
                let mut lines = vec![COPY_HEADER.to_string()];
                for (r, err) in &app.rows {
                    let status = match r.state {
                        2 => format!("✗ {err}"),
                        1 => "✓".into(),
                        _ => "●".into(),
                    };
                    lines.push(format!(
                        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        r.no, r.file_name, r.page_count, r.upload, r.ready, r.download, r.convert, r.first, r.full,
                        r.e2e, status
                    ));
                }
                app.status_text = match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(lines.join("\n"))) {
                    Ok(_) => "已复制到剪贴板".into(),
                    Err(e) => format!("复制失败: {e}"),
                };
            }
            Task::none()
        }
        Message::ExportCsv => {
            if app.rows.is_empty() {
                app.status_text = "暂无结果可导出".into();
            } else {
                let mut csv = format!("{CSV_HEADER}\n");
                for (i, (r, err)) in app.rows.iter().enumerate() {
                    let status = match r.state {
                        2 => "失败",
                        1 => "完成",
                        _ => "预热",
                    };
                    csv.push_str(&format!(
                        "{},{},{},{},{},{},{},{},{},{},{},{}\n",
                        i + 1,
                        r.file_name,
                        r.page_count,
                        r.upload,
                        r.ready,
                        r.download,
                        r.convert,
                        r.first,
                        r.full,
                        r.e2e,
                        status,
                        err.replace(',', "，")
                    ));
                }
                if let Some(path) = rfd::FileDialog::new().set_file_name("core-flow.csv").save_file() {
                    app.status_text = match std::fs::write(&path, csv) {
                        Ok(_) => format!("已导出 {}", path.display()),
                        Err(e) => format!("写入失败: {e}"),
                    };
                }
            }
            Task::none()
        }
        Message::StartTest => {
            app.start_test();
            Task::none()
        }
        Message::StopTest => {
            app.handle.stop_flag.store(true, Ordering::Relaxed);
            Task::none()
        }
        Message::PollTick => {
            if !app.running {
                return Task::none();
            }
            let handle = app.handle.clone();
            let run_flag = app.run_flag.clone();
            let last_log_len = app.last_log_len;
            Task::future(async move {
                let (logs, rows, summary) = {
                    let st = handle.status.lock().await;
                    (st.logs.clone(), st.rows.clone(), st.summary.clone())
                };
                let log_len = logs.len();
                let log = if log_len != last_log_len { Some(logs.join("\n")) } else { None };
                let success = rows.iter().filter(|r| r.success).count();
                let total = rows.len();
                let ui_rows: Vec<_> = rows.iter().enumerate().map(|(i, r)| model::to_ui_row(r, i + 1)).collect();
                let running = run_flag.load(Ordering::Relaxed);
                Message::Polled(Box::new(Snapshot {
                    running,
                    rows: ui_rows,
                    log,
                    log_len,
                    success,
                    total,
                    summary,
                }))
            })
        }
        Message::Polled(s) => {
            app.rows = s.rows;
            app.last_log_len = s.log_len;
            if let Some(t) = s.log {
                app.log_text = t;
            }
            if s.running {
                app.status_text = format!("运行中… {}/{}", s.success, s.total);
            } else {
                if let Some(sm) = &s.summary {
                    if sm.total > 0 {
                        app.status_text =
                            format!("{}/{} 成功 · 吞吐 {:.1} 件/min", sm.success, sm.total, sm.files_per_min);
                    }
                }
                app.running = false;
            }
            Task::none()
        }
        Message::ResultRowHover(i) => {
            app.hovered_result_row = i;
            Task::none()
        }
        Message::FileRowHover(i) => {
            app.hovered_file_row = i;
            Task::none()
        }
        Message::Modifiers(m) => {
            app.modifiers = m;
            Task::none()
        }
        Message::RowClick(i) => {
            if let Some(f) = app.files.get_mut(i) {
                f.checked = !f.checked;
                let new_state = f.checked;
                // Shift+点击:把锚点行到当前行统一成本次点击后的勾选状态
                if app.modifiers.shift() {
                    if let Some(a) = app.click_anchor {
                        let (lo, hi) = (a.min(i), a.max(i));
                        for f in &mut app.files[lo..=hi] {
                            f.checked = new_state;
                        }
                    }
                }
                app.click_anchor = Some(i);
                app.sync_all_checked();
            }
            Task::none()
        }
    }
}

fn subscription(app: &App) -> Subscription<Message> {
    let tick = if app.running {
        iced::time::every(Duration::from_millis(500)).map(|_| Message::PollTick)
    } else {
        Subscription::none()
    };
    // 修饰键跟踪(文件列表 Shift+点击范围多选)
    let mods = iced::event::listen_with(|event, _status, _id| match event {
        iced::event::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) => {
            Some(Message::Modifiers(m))
        }
        _ => None,
    });
    Subscription::batch([tick, mods])
}

fn title_of(_app: &App) -> String {
    "x-doc 核心链路压测".to_string()
}

fn theme_of(_app: &App) -> iced::Theme {
    iced::Theme::Light
}

fn main() -> iced::Result {
    iced::application(App::new, update, view::view)
        .title(title_of)
        .window(window::Settings {
            size: Size::new(1280.0, 800.0),
            min_size: Some(Size::new(1060.0, 600.0)),
            ..Default::default()
        })
        .theme(theme_of)
        .subscription(subscription)
        .run()
}
