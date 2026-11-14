//! The window: volumes, explorer tree, largest files, git repos, inspector and status bar.

use crate::scan::{self, GitInfo, Kind, Progress, SizeMode, Tree, Volume};
use crate::{cache, tray};
use eframe::egui;
use egui_extras::{Column, TableBuilder, TableRow};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// ⌘O on macOS, Ctrl+O elsewhere.
const OPEN_KEY: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
const RESCAN_KEY: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::R);
const STOP_KEY: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Period);
const SELECT_ALL_KEY: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::A);
const LARGEST_LIMIT: usize = 500;
const STALE_SECS: i64 = 365 * 86400;
const ROW_HEIGHT: f32 = 22.0;
const VOLUME_REFRESH: Duration = Duration::from_secs(10);

const GREEN: egui::Color32 = egui::Color32::from_rgb(0x3C, 0xB4, 0x6E);
const ORANGE: egui::Color32 = egui::Color32::from_rgb(0xE8, 0x9A, 0x3C);
const RED: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x4E, 0x4E);
const PURPLE: egui::Color32 = egui::Color32::from_rgb(0xA5, 0x6E, 0xE8);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Volumes,
    Tree,
    Largest,
    Repos,
    Bin,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Col {
    Name,
    Size,
    Percent,
    Items,
    Activity,
    Modified,
    Perms,
    Owner,
    Git,
    Path,
    Branch,
    Changes,
    GitDir,
    Remote,
}

impl Col {
    fn label(self) -> &'static str {
        match self {
            Col::Name => "Name",
            Col::Size => "Size",
            Col::Percent => "% of parent",
            Col::Items => "Items",
            Col::Activity => "Last activity",
            Col::Modified => "Modified",
            Col::Perms => "Permissions",
            Col::Owner => "Owner",
            Col::Git => "Git",
            Col::Path => "Folder",
            Col::Branch => "Branch",
            Col::Changes => "Changes",
            Col::GitDir => ".git size",
            Col::Remote => "Remote",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Col::Size => "On disk: allocated blocks, what deleting frees. Logical: byte length.",
            Col::Items => "Files and folders inside",
            Col::Activity => "Newest modification anywhere inside",
            Col::Modified => "The item's own modification date",
            Col::Perms => {
                if cfg!(windows) {
                    "d=folder r=read-only h=hidden s=system a=archive"
                } else {
                    "Unix mode; dimmed when you cannot write to it"
                }
            }
            Col::Git => "Branch · changed files · last commit",
            _ => "",
        }
    }
}

#[derive(Clone, Copy)]
struct Sort {
    col: Col,
    desc: bool,
}

enum Action {
    Toggle(u32),
    Select { idx: u32, toggle: bool, range: bool },
    Reveal(u32),
    CopyPaths,
    Terminal(u32),
    ScanFolder(u32),
    ScanPath(PathBuf),
    AskTrash,
    AskMove,
}

#[derive(Clone)]
enum OpKind {
    Trash,
    Delete,
    Move(PathBuf),
}

struct OpResult {
    done: Vec<u32>,
    errors: Vec<String>,
}

struct OpJob {
    rx: Receiver<OpResult>,
    label: String,
    files: Arc<AtomicUsize>,
}

struct BinRow {
    item: scan::bin::Item,
    size: Option<u64>,
    items: Option<u64>,
}

struct BinOp {
    rx: Receiver<Result<(), String>>,
    label: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BinConfirm {
    Purge,
    Empty,
}

struct ScanJob {
    progress: Arc<Progress>,
    rx: Receiver<Option<Tree>>,
    started: Instant,
}

pub struct App {
    tree: Option<Tree>,
    root_path: Option<PathBuf>,
    root_volume: Option<Volume>,
    scan: Option<ScanJob>,
    /// Generation of the tree, so late git results for an old scan are dropped.
    generation: u64,
    elapsed: f64,
    /// Unix seconds when the shown tree was scanned.
    scanned_at: i64,
    /// The tree came from the cache of an earlier run, not from a scan in this one.
    restored: bool,
    restore_rx: Option<Receiver<Option<cache::Saved>>>,
    tray: Option<tray::Tray>,
    progress_snapshot: (usize, u64, String),
    cancelled: bool,
    git_rx: Option<Receiver<(u64, u32, GitInfo)>>,
    git_done: usize,
    largest: Vec<u32>,
    repos: Vec<u32>,
    expanded: HashSet<u32>,
    rows: Vec<(u32, u16)>,
    rows_dirty: bool,
    /// Selected nodes; `anchor` is the last clicked one, shown in the inspector.
    selected: BTreeSet<u32>,
    anchor: Option<u32>,
    sort: Sort,
    sort_largest: Sort,
    sort_repos: Sort,
    tab: Tab,
    size_mode: SizeMode,
    folders_only: bool,
    show_inspector: bool,
    theme: &'static str,
    owners: HashMap<u32, String>,
    confirm_trash: bool,
    op: Option<OpJob>,
    volumes: Vec<Volume>,
    volumes_at: Option<Instant>,
    bin: Vec<BinRow>,
    bin_rx: Option<Receiver<(usize, u64, u64)>>,
    bin_loaded: bool,
    bin_error: String,
    bin_sel: BTreeSet<usize>,
    bin_op: Option<BinOp>,
    bin_confirm: Option<BinConfirm>,
    message: String,
}

pub fn run(start: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App {
        tree: None,
        root_path: None,
        root_volume: None,
        scan: None,
        generation: 0,
        elapsed: 0.0,
        scanned_at: 0,
        restored: false,
        restore_rx: None,
        tray: None,
        progress_snapshot: (0, 0, String::new()),
        cancelled: false,
        git_rx: None,
        git_done: 0,
        largest: Vec::new(),
        repos: Vec::new(),
        expanded: HashSet::new(),
        rows: Vec::new(),
        rows_dirty: false,
        selected: BTreeSet::new(),
        anchor: None,
        sort: Sort {
            col: Col::Size,
            desc: true,
        },
        sort_largest: Sort {
            col: Col::Size,
            desc: true,
        },
        sort_repos: Sort {
            col: Col::Size,
            desc: true,
        },
        tab: Tab::Volumes,
        size_mode: SizeMode::OnDisk,
        folders_only: false,
        show_inspector: true,
        theme: "system",
        owners: HashMap::new(),
        confirm_trash: false,
        op: None,
        volumes: Vec::new(),
        volumes_at: None,
        bin: Vec::new(),
        bin_rx: None,
        bin_loaded: false,
        bin_error: String::new(),
        bin_sel: BTreeSet::new(),
        bin_op: None,
        bin_confirm: None,
        message: String::new(),
    };
    if let Some(p) = start {
        app.start_scan(p);
    } else {
        // A big cache takes a moment to read: the window opens meanwhile.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(cache::load());
        });
        app.restore_rx = Some(rx);
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("RustDU")
            .with_inner_size([1500.0, 900.0])
            .with_min_inner_size([900.0, 500.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/disk-explorer.png"))
                    .expect("bundled icon is a valid png"),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "RustDU",
        opts,
        Box::new(move |cc| {
            apply_theme(&cc.egui_ctx, app.theme);
            app.tray = tray::install(cc);
            Ok(Box::new(app))
        }),
    )?;
    Ok(())
}

// MARK: scanning and state

impl App {
    fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    fn busy(&self) -> bool {
        self.scanning() || self.op.is_some() || self.bin_op.is_some()
    }

    fn choose_folder(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose a folder to scan");
        if let Some(p) = &self.root_path {
            dialog = dialog.set_directory(p);
        }
        if let Some(p) = dialog.pick_folder() {
            self.start_scan(p);
        }
    }

    fn start_scan(&mut self, path: PathBuf) {
        if self.op.is_some() {
            self.message = "Wait for the current file operation to finish.".into();
            return;
        }
        self.stop_scan();
        self.restore_rx = None;
        self.restored = false;
        self.tree = None;
        self.git_rx = None;
        self.git_done = 0;
        self.largest.clear();
        self.repos.clear();
        self.expanded.clear();
        self.rows.clear();
        self.owners.clear();
        self.selected.clear();
        self.anchor = None;
        self.confirm_trash = false;
        self.cancelled = false;
        self.message.clear();
        self.generation += 1;
        self.progress_snapshot = (0, 0, path.display().to_string());
        self.root_volume = scan::volume_of(&path);
        self.root_path = Some(path.clone());
        self.tab = Tab::Tree;
        let progress = Arc::new(Progress::default());
        let (tx, rx) = mpsc::channel();
        let worker = Arc::clone(&progress);
        std::thread::spawn(move || {
            let tree = scan::scan(&path, &worker);
            let _ = tx.send(tree);
        });
        self.scan = Some(ScanJob {
            progress,
            rx,
            started: Instant::now(),
        });
    }

    fn stop_scan(&mut self) {
        if let Some(job) = &self.scan {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn refresh_volumes(&mut self, force: bool) {
        if force || self.volumes_at.is_none_or(|t| t.elapsed() > VOLUME_REFRESH) {
            self.volumes = scan::volumes();
            self.volumes_at = Some(Instant::now());
            if let Some(p) = &self.root_path {
                self.root_volume = scan::volume_of(p);
            }
        }
    }

    fn poll(&mut self) {
        if let Some(job) = &self.scan {
            self.elapsed = job.started.elapsed().as_secs_f64();
            let p = &job.progress;
            self.progress_snapshot = (
                p.items.load(Ordering::Relaxed),
                p.bytes.load(Ordering::Relaxed),
                p.current.lock().map(|s| s.clone()).unwrap_or_default(),
            );
            if let Ok(result) = job.rx.try_recv() {
                self.cancelled = job.progress.cancelled();
                self.scan = None;
                match result {
                    Some(tree) => {
                        self.scanned_at = now();
                        self.finish(tree);
                        self.save_cache();
                    }
                    None => self.message = "That folder could not be read.".into(),
                }
            }
        }
        if let Some(rx) = &self.restore_rx
            && let Ok(saved) = rx.try_recv()
        {
            self.restore_rx = None;
            if let Some(saved) = saved {
                self.restore(saved);
            }
        }
        if let Some(rx) = &self.git_rx {
            let mut got = Vec::new();
            while let Ok(m) = rx.try_recv() {
                got.push(m);
            }
            for (generation, idx, info) in got {
                if generation != self.generation {
                    continue;
                }
                if let Some(t) = &mut self.tree
                    && let Some(n) = t.nodes.get_mut(idx as usize)
                {
                    n.git = Some(info);
                    self.git_done += 1;
                    if matches!(self.sort.col, Col::Git) {
                        self.rows_dirty = true;
                    }
                }
            }
        }
        self.poll_op();
        self.poll_bin();
    }

    fn finish(&mut self, tree: Tree) {
        self.largest = tree.largest_files(LARGEST_LIMIT);
        self.repos = tree.git_repos();
        // Probe repositories in the background; results arrive through git_rx.
        let work: Vec<(u32, PathBuf)> = self.repos.iter().map(|&i| (i, tree.path(i))).collect();
        let (tx, rx) = mpsc::channel();
        let generation = self.generation;
        std::thread::spawn(move || {
            use rayon::prelude::*;
            work.into_par_iter().for_each(|(idx, path)| {
                let _ = tx.send((generation, idx, scan::git_info(&path)));
            });
        });
        self.git_rx = Some(rx);
        self.tree = Some(tree);
        self.sort_flat(Tab::Largest);
        self.sort_flat(Tab::Repos);
        self.rows_dirty = true;
        self.refresh_volumes(true);
    }

    /// Show the scan kept from an earlier run; `start_scan` drops the pending load, so nothing is replaced.
    fn restore(&mut self, saved: cache::Saved) {
        self.generation += 1;
        self.cancelled = saved.partial;
        self.scanned_at = saved.scanned_at;
        self.restored = true;
        self.root_volume = scan::volume_of(&saved.tree.root_path);
        self.root_path = Some(saved.tree.root_path.clone());
        self.tab = Tab::Tree;
        self.finish(saved.tree);
    }

    fn save_cache(&self) {
        if let Some(t) = &self.tree {
            cache::save(cache::encode(t, self.scanned_at, self.cancelled));
        }
    }

    /// The rows of the active tab, in display order: the range for shift-click.
    fn current_list(&self) -> Vec<u32> {
        match self.tab {
            Tab::Tree => self.rows.iter().map(|r| r.0).collect(),
            Tab::Largest => self.largest.clone(),
            Tab::Repos => self.repos.clone(),
            Tab::Volumes | Tab::Bin => Vec::new(),
        }
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        if let Action::ScanPath(p) = action {
            self.start_scan(p);
            return;
        }
        let Some(tree) = &self.tree else { return };
        match action {
            Action::Toggle(i) => {
                if !self.expanded.remove(&i) {
                    self.expanded.insert(i);
                }
                self.rows_dirty = true;
            }
            Action::Select { idx, toggle, range } => {
                if range && let Some(a) = self.anchor {
                    let list = self.current_list();
                    let pos = |x: u32| list.iter().position(|&v| v == x);
                    if let (Some(i), Some(j)) = (pos(a), pos(idx)) {
                        if !toggle {
                            self.selected.clear();
                        }
                        let (lo, hi) = if i <= j { (i, j) } else { (j, i) };
                        self.selected.extend(list[lo..=hi].iter().copied());
                        return;
                    }
                }
                if toggle {
                    if !self.selected.remove(&idx) {
                        self.selected.insert(idx);
                    }
                } else {
                    self.selected.clear();
                    self.selected.insert(idx);
                }
                self.anchor = Some(idx);
            }
            Action::Reveal(i) => reveal(&tree.path(i)),
            Action::CopyPaths => {
                let text: Vec<String> = self
                    .selected
                    .iter()
                    .map(|&i| tree.path(i).display().to_string())
                    .collect();
                ctx.copy_text(text.join("\n"));
            }
            Action::Terminal(i) => {
                let p = tree.path(i);
                open_terminal(if tree.nodes[i as usize].is_dir() {
                    &p
                } else {
                    p.parent().unwrap_or(&p)
                });
            }
            Action::ScanFolder(i) => {
                let p = tree.path(i);
                self.start_scan(p);
            }
            Action::AskTrash => {
                if self.op.is_none() && self.selected.iter().any(|&i| i != 0) {
                    self.confirm_trash = true;
                }
            }
            Action::AskMove => self.ask_move(),
            Action::ScanPath(_) => {}
        }
    }

    // MARK: file operations

    /// Selected items that are not the root and not inside another selected item.
    fn operation_targets(&self) -> Vec<(u32, PathBuf)> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        self.selected
            .iter()
            .copied()
            .filter(|&i| i != 0)
            .filter(|&i| !self.selected.iter().any(|&o| o != i && tree.is_under(i, o)))
            .map(|i| (i, tree.path(i)))
            .collect()
    }

    fn ask_move(&mut self) {
        let targets = self.operation_targets();
        if targets.is_empty() || self.op.is_some() {
            return;
        }
        let title = if targets.len() == 1 {
            format!(
                "Move “{}” to…",
                targets[0]
                    .1
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
        } else {
            format!("Move {} items to…", targets.len())
        };
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(p) = &self.root_path {
            dialog = dialog.set_directory(p);
        }
        if let Some(dest) = dialog.pick_folder() {
            if targets.iter().any(|(_, p)| dest.starts_with(p)) {
                self.message = "The destination is inside an item being moved.".into();
                return;
            }
            self.start_op(OpKind::Move(dest));
        }
    }

    fn start_op(&mut self, kind: OpKind) {
        let targets = self.operation_targets();
        if targets.is_empty() {
            return;
        }
        let what = if targets.len() == 1 {
            targets[0]
                .1
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        } else {
            format!("{} items", targets.len())
        };
        let label = match &kind {
            OpKind::Trash => format!("Moving {what} to the Trash…"),
            OpKind::Delete => format!("Deleting {what}…"),
            OpKind::Move(d) => format!("Moving {what} to {}…", d.display()),
        };
        let files = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&files);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut result = OpResult {
                done: Vec::new(),
                errors: Vec::new(),
            };
            for (idx, path) in targets {
                let r = match &kind {
                    OpKind::Trash => scan::ops::trash(&path),
                    OpKind::Delete => scan::ops::delete_permanently(&path),
                    OpKind::Move(dest) => scan::ops::move_into(&path, dest, &counter).map(|_| ()),
                };
                match r {
                    Ok(()) => result.done.push(idx),
                    Err(e) => result.errors.push(format!("{}: {e}", path.display())),
                }
            }
            let _ = tx.send(result);
        });
        self.op = Some(OpJob { rx, label, files });
        self.confirm_trash = false;
        self.selected.clear();
        self.anchor = None;
    }

    fn poll_op(&mut self) {
        let Some(job) = &self.op else { return };
        let Ok(result) = job.rx.try_recv() else {
            return;
        };
        let label = job.label.trim_end_matches('…').to_string();
        self.op = None;
        if let Some(tree) = &mut self.tree {
            for &idx in &result.done {
                tree.remove(idx);
            }
            let t = &*tree;
            let gone = |i: u32| result.done.iter().any(|&d| t.is_under(i, d));
            self.largest.retain(|&i| !gone(i));
            self.repos.retain(|&i| !gone(i));
            self.owners.clear();
        }
        if !result.done.is_empty() {
            self.save_cache();
        }
        self.message = match result.errors.first() {
            None => format!("{label}: done"),
            Some(e) => format!(
                "{label}: {} done, {} failed. {e}",
                result.done.len(),
                result.errors.len()
            ),
        };
        self.rows_dirty = true;
        self.refresh_volumes(true);
    }

    // MARK: Trash

    fn load_bin(&mut self) {
        self.bin_sel.clear();
        self.bin_rx = None;
        self.bin_error.clear();
        self.bin_loaded = true;
        let items = match scan::bin::list() {
            Ok(i) => i,
            Err(e) => {
                self.bin_error = e;
                self.bin.clear();
                return;
            }
        };
        self.bin = items
            .into_iter()
            .map(|item| BinRow {
                item,
                size: None,
                items: None,
            })
            .collect();
        // Sizes come from walking each item in the background.
        let work: Vec<(usize, PathBuf)> = self
            .bin
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.item.path.clone().map(|p| (i, p)))
            .collect();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let progress = Progress::default();
            for (i, path) in work {
                if let Some(t) = scan::scan(&path, &progress) {
                    let _ = tx.send((i, t.root().allocated, t.root().items().max(1)));
                }
            }
        });
        self.bin_rx = Some(rx);
    }

    fn poll_bin(&mut self) {
        if let Some(rx) = &self.bin_rx {
            let mut got = Vec::new();
            while let Ok(m) = rx.try_recv() {
                got.push(m);
            }
            let mut changed = false;
            for (i, bytes, items) in got {
                if let Some(r) = self.bin.get_mut(i) {
                    r.size = Some(bytes);
                    r.items = Some(items);
                    changed = true;
                }
            }
            if changed {
                self.bin
                    .sort_by_key(|r| std::cmp::Reverse(r.size.unwrap_or(0)));
                self.bin_sel.clear();
            }
        }
        if let Some(op) = &self.bin_op
            && let Ok(result) = op.rx.try_recv()
        {
            let label = op.label.trim_end_matches('…').to_string();
            self.message = match result {
                Ok(()) => format!("{label}: done"),
                Err(e) => format!("{label}: {e}"),
            };
            self.bin_op = None;
            self.load_bin();
            self.refresh_volumes(true);
        }
    }

    fn bin_targets(&self, all: bool) -> Vec<scan::bin::Item> {
        self.bin
            .iter()
            .enumerate()
            .filter(|(i, _)| all || self.bin_sel.contains(i))
            .map(|(_, r)| r.item.clone())
            .collect()
    }

    fn start_bin_op(&mut self, restore: bool, all: bool) {
        let items = self.bin_targets(all);
        if items.is_empty() || self.bin_op.is_some() {
            return;
        }
        let what = if items.len() == 1 {
            items[0].name.clone()
        } else {
            format!("{} items", items.len())
        };
        let label = if restore {
            format!("Restoring {what}…")
        } else if all {
            "Emptying the Trash…".to_string()
        } else {
            format!("Deleting {what} permanently…")
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let r = if restore {
                scan::bin::restore(&items)
            } else {
                scan::bin::purge(&items)
            };
            let _ = tx.send(r);
        });
        self.bin_op = Some(BinOp { rx, label });
        self.bin_confirm = None;
    }

    fn bin_view(&mut self, ui: &mut egui::Ui) {
        if !self.bin_loaded {
            self.load_bin();
        }
        let now = now();
        let known: u64 = self.bin.iter().filter_map(|r| r.size).sum();
        let sizing = self
            .bin
            .iter()
            .any(|r| r.size.is_none() && r.item.path.is_some());
        let busy = self.bin_op.is_some();
        let has_sel = !self.bin_sel.is_empty();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading(if cfg!(windows) {
                "Recycle Bin"
            } else {
                "Trash"
            });
            ui.monospace(format!(
                "{} items · {}",
                count(self.bin.len() as u64),
                size(known)
            ));
            if sizing {
                ui.spinner();
                ui.weak("measuring…");
            }
            if ui.small_button("Refresh").clicked() {
                self.load_bin();
            }
            ui.separator();
            if ui
                .add_enabled(
                    scan::bin::CAN_RESTORE && has_sel && !busy,
                    egui::Button::new("Restore"),
                )
                .on_disabled_hover_text(if scan::bin::CAN_RESTORE {
                    "Select items to restore"
                } else {
                    "On macOS, use Put Back in the Finder: Reveal opens the item in the Trash"
                })
                .clicked()
            {
                self.start_bin_op(true, false);
            }
            if ui
                .add_enabled(has_sel && !busy, egui::Button::new("Delete permanently…"))
                .clicked()
            {
                self.bin_confirm = Some(BinConfirm::Purge);
            }
            if ui
                .add_enabled(!self.bin.is_empty() && !busy, egui::Button::new("Empty…"))
                .clicked()
            {
                self.bin_confirm = Some(BinConfirm::Empty);
            }
            if let Some(op) = &self.bin_op {
                ui.spinner();
                ui.weak(&op.label);
            }
        });
        if !self.bin_error.is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, &self.bin_error);
            return;
        }
        if self.bin.is_empty() {
            ui.add_space(20.0);
            ui.weak("The Trash is empty.");
            return;
        }
        ui.weak(
            "Click selects, ⌘/Ctrl-click adds. Right-click a row for Reveal, Restore and Delete.",
        );
        ui.add_space(4.0);
        let max = self
            .bin
            .iter()
            .filter_map(|r| r.size)
            .max()
            .unwrap_or(1)
            .max(1);
        let mut clicks: Vec<(usize, bool)> = Vec::new();
        let mut reveal_path: Option<PathBuf> = None;
        let mut menu_action: Option<(usize, BinConfirm, bool)> = None; // (row, kind, restore)
        let Self { bin, bin_sel, .. } = self;
        TableBuilder::new(ui)
            .id_salt("bin")
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(320.0).at_least(160.0).clip(true))
            .column(Column::initial(170.0).at_least(120.0))
            .column(Column::initial(80.0).at_least(60.0))
            .column(Column::initial(120.0).at_least(90.0))
            .column(Column::remainder().at_least(200.0).clip(true))
            .min_scrolled_height(0.0)
            .header(24.0, |mut h| {
                for label in ["Name", "Size", "Items", "Deleted", "Original location"] {
                    h.col(|ui| {
                        ui.label(egui::RichText::new(label).strong());
                    });
                }
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, bin.len(), |mut row| {
                    let i = row.index();
                    let r = &bin[i];
                    let in_sel = bin_sel.contains(&i);
                    row.set_selected(in_sel);
                    row.col(|ui| {
                        let is_dir = r.item.path.as_ref().is_some_and(|p| p.is_dir());
                        ui.label(egui::RichText::new(if is_dir { "📁" } else { "📄" }).color(
                            if is_dir {
                                folder_color(ui)
                            } else {
                                ui.visuals().weak_text_color()
                            },
                        ));
                        ui.add(egui::Label::new(&r.item.name).truncate());
                    });
                    row.col(|ui| match r.size {
                        Some(b) => size_cell(ui, b, b as f64 / max as f64, true),
                        None => {
                            ui.weak("…");
                        }
                    });
                    row.col(|ui| {
                        if let Some(n) = r.items {
                            ui.weak(count(n));
                        }
                    });
                    row.col(|ui| date_cell(ui, r.item.deleted, now));
                    row.col(|ui| {
                        let text = r
                            .item
                            .original
                            .as_ref()
                            .map_or("—".to_string(), |p| p.display().to_string());
                        ui.add(egui::Label::new(egui::RichText::new(text).weak()).truncate());
                    });
                    let resp = row.response();
                    let m = resp.ctx.input(|i| i.modifiers);
                    if resp.clicked() {
                        clicks.push((i, m.command));
                    }
                    if resp.secondary_clicked() && !in_sel {
                        clicks.push((i, false));
                    }
                    resp.context_menu(|ui| {
                        if let Some(p) = &r.item.path
                            && ui.button("Reveal").clicked()
                        {
                            reveal_path = Some(p.clone());
                            ui.close();
                        }
                        if ui
                            .add_enabled(scan::bin::CAN_RESTORE, egui::Button::new("Restore"))
                            .clicked()
                        {
                            menu_action = Some((i, BinConfirm::Purge, true));
                            ui.close();
                        }
                        if ui.button("Delete permanently…").clicked() {
                            menu_action = Some((i, BinConfirm::Purge, false));
                            ui.close();
                        }
                    });
                });
            });
        for (i, toggle) in clicks {
            if toggle {
                if !self.bin_sel.remove(&i) {
                    self.bin_sel.insert(i);
                }
            } else {
                self.bin_sel.clear();
                self.bin_sel.insert(i);
            }
        }
        if let Some(p) = reveal_path {
            reveal(&p);
        }
        if let Some((i, kind, restore)) = menu_action {
            if !self.bin_sel.contains(&i) {
                self.bin_sel.clear();
                self.bin_sel.insert(i);
            }
            if restore {
                self.start_bin_op(true, false);
            } else {
                self.bin_confirm = Some(kind);
            }
        }
    }

    fn bin_dialog(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.bin_confirm else { return };
        let all = kind == BinConfirm::Empty;
        let targets: Vec<&BinRow> = self
            .bin
            .iter()
            .enumerate()
            .filter(|(i, _)| all || self.bin_sel.contains(i))
            .map(|(_, r)| r)
            .collect();
        if targets.is_empty() {
            self.bin_confirm = None;
            return;
        }
        let total: u64 = targets.iter().filter_map(|r| r.size).sum();
        let title = if all {
            format!(
                "Empty the {}?",
                if cfg!(windows) {
                    "Recycle Bin"
                } else {
                    "Trash"
                }
            )
        } else if targets.len() == 1 {
            format!("Delete “{}” permanently?", targets[0].item.name)
        } else {
            format!("Delete {} items permanently?", targets.len())
        };
        let mut decision: Option<bool> = None;
        let modal = egui::Modal::new(egui::Id::unique("bin-confirm")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(title);
            ui.label(format!(
                "{} items · {} · cannot be undone",
                count(targets.len() as u64),
                size(total)
            ));
            for r in targets.iter().take(8) {
                ui.add(egui::Label::new(egui::RichText::new(&r.item.name).weak()).truncate());
            }
            if targets.len() > 8 {
                ui.weak(format!("… and {} more", targets.len() - 8));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    decision = Some(false);
                }
                let b = egui::Button::new(
                    egui::RichText::new("Delete permanently").color(egui::Color32::WHITE),
                )
                .fill(egui::Color32::from_rgb(0xC9, 0x3C, 0x3C));
                if ui.add(b).clicked() {
                    decision = Some(true);
                }
            });
        });
        if modal.should_close() {
            decision = Some(false);
        }
        match decision {
            Some(true) => self.start_bin_op(false, all),
            Some(false) => self.bin_confirm = None,
            None => {}
        }
    }

    // MARK: ordering

    fn rebuild_rows(&mut self) {
        self.rows.clear();
        let Some(tree) = &self.tree else { return };
        let mut stack: Vec<(u32, u16)> =
            sorted_children(tree, 0, self.sort, self.size_mode, self.folders_only)
                .into_iter()
                .rev()
                .map(|i| (i, 0))
                .collect();
        while let Some((idx, depth)) = stack.pop() {
            self.rows.push((idx, depth));
            if self.expanded.contains(&idx) {
                for c in sorted_children(tree, idx, self.sort, self.size_mode, self.folders_only)
                    .into_iter()
                    .rev()
                {
                    stack.push((c, depth + 1));
                }
            }
        }
        self.rows_dirty = false;
    }

    fn sort_flat(&mut self, tab: Tab) {
        let Some(tree) = &self.tree else { return };
        let (list, sort) = match tab {
            Tab::Largest => (&mut self.largest, self.sort_largest),
            Tab::Repos => (&mut self.repos, self.sort_repos),
            Tab::Tree | Tab::Volumes | Tab::Bin => return,
        };
        let mode = self.size_mode;
        list.sort_by(|&a, &b| compare(tree, sort, mode, a, b));
    }

    fn click_sort(&mut self, tab: Tab, col: Col) {
        let sort = match tab {
            Tab::Tree => &mut self.sort,
            Tab::Largest => &mut self.sort_largest,
            Tab::Repos => &mut self.sort_repos,
            Tab::Volumes | Tab::Bin => return,
        };
        if sort.col == col {
            sort.desc = !sort.desc;
        } else {
            *sort = Sort {
                col,
                desc: !matches!(
                    col,
                    Col::Name | Col::Perms | Col::Owner | Col::Path | Col::Branch | Col::Remote
                ),
            };
        }
        match tab {
            Tab::Tree => self.rows_dirty = true,
            _ => self.sort_flat(tab),
        }
    }

    fn size_mode_changed(&mut self) {
        self.rows_dirty = true;
        self.sort_flat(Tab::Largest);
        self.sort_flat(Tab::Repos);
    }
}

fn sorted_children(
    tree: &Tree,
    idx: u32,
    sort: Sort,
    mode: SizeMode,
    folders_only: bool,
) -> Vec<u32> {
    let mut kids: Vec<u32> = tree.nodes[idx as usize]
        .children
        .iter()
        .copied()
        .filter(|&c| !folders_only || tree.nodes[c as usize].is_dir())
        .collect();
    kids.sort_by(|&a, &b| compare(tree, sort, mode, a, b));
    kids
}

fn compare(tree: &Tree, sort: Sort, mode: SizeMode, a: u32, b: u32) -> std::cmp::Ordering {
    let (na, nb) = (&tree.nodes[a as usize], &tree.nodes[b as usize]);
    let git_date = |n: &scan::Node| {
        n.git
            .as_ref()
            .and_then(|g| g.last_commit)
            .unwrap_or(i64::MIN)
    };
    let git_changes = |n: &scan::Node| {
        n.git
            .as_ref()
            .and_then(|g| g.changed)
            .map_or(-1, |c| c as i64)
    };
    let git_dir = |i: u32| {
        tree.git_dir(i)
            .map_or(0, |d| tree.nodes[d as usize].size(mode))
    };
    let ord = match sort.col {
        Col::Name => na.name.to_lowercase().cmp(&nb.name.to_lowercase()),
        Col::Size | Col::Percent => na.size(mode).cmp(&nb.size(mode)),
        Col::Items => na.items().cmp(&nb.items()),
        Col::Activity => na.newest.cmp(&nb.newest),
        Col::Modified => na.modified.cmp(&nb.modified),
        Col::Perms => scan::perm_string(na.kind, na.mode, na.attrs)
            .cmp(&scan::perm_string(nb.kind, nb.mode, nb.attrs)),
        Col::Owner => (na.uid, na.gid).cmp(&(nb.uid, nb.gid)),
        Col::Git => git_date(na).cmp(&git_date(nb)),
        Col::Path => tree.path(a).cmp(&tree.path(b)),
        Col::Branch => {
            let b_of = |n: &scan::Node| {
                n.git
                    .as_ref()
                    .map(|g| g.branch.to_lowercase())
                    .unwrap_or_default()
            };
            b_of(na).cmp(&b_of(nb))
        }
        Col::Changes => git_changes(na).cmp(&git_changes(nb)),
        Col::GitDir => git_dir(a).cmp(&git_dir(b)),
        Col::Remote => {
            let r_of =
                |n: &scan::Node| n.git.as_ref().map(|g| g.remote.clone()).unwrap_or_default();
            r_of(na).cmp(&r_of(nb))
        }
    };
    let ord = ord.then_with(|| na.name.cmp(&nb.name));
    if sort.desc { ord.reverse() } else { ord }
}

// MARK: frame

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if ctx.input_mut(|i| i.consume_shortcut(&OPEN_KEY)) {
            self.choose_folder();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&RESCAN_KEY))
            && !self.scanning()
            && let Some(p) = self.root_path.clone()
        {
            self.start_scan(p);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&STOP_KEY)) {
            self.stop_scan();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SELECT_ALL_KEY)) && self.tree.is_some() {
            self.selected = self.current_list().into_iter().collect();
        }
        let delete_pressed = ctx.input(|i| {
            !i.modifiers.any()
                && (i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        });
        if delete_pressed && !self.confirm_trash {
            self.apply(Action::AskTrash, ctx);
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(dir) = dropped.into_iter().find(|p| p.is_dir()) {
            self.start_scan(dir);
        }
        if self.tab == Tab::Volumes {
            self.refresh_volumes(false);
        }
        if self.tab != Tab::Bin {
            // Reload next time the Trash is opened.
            self.bin_loaded = false;
        }
        if self.rows_dirty {
            self.rebuild_rows();
        }
        let closing = ctx.input(|i| i.viewport().close_requested());
        if tray::quit_requested() {
            if !closing {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else if closing && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::hide_app();
        }
        if self.busy()
            || self.git_done < self.repos.len()
            || self.bin_rx.is_some()
            || self.restore_rx.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(150));
        } else if self.tab == Tab::Volumes {
            ctx.request_repaint_after(VOLUME_REFRESH);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.top_bar(ui);
        self.status_bar(ui);
        if self.show_inspector
            && self.tree.is_some()
            && !matches!(self.tab, Tab::Volumes | Tab::Bin)
        {
            self.inspector(ui);
        }
        let mut actions: Vec<Action> = Vec::new();
        egui::CentralPanel::default().show(ui, |ui| {
            if self.tab == Tab::Volumes {
                self.volumes_view(ui, &mut actions);
                return;
            }
            if self.tab == Tab::Bin {
                self.bin_view(ui);
                return;
            }
            if self.tree.is_none() {
                self.empty_state(ui);
                return;
            }
            self.header(ui);
            ui.separator();
            match self.tab {
                Tab::Tree => self.tree_table(ui, &mut actions),
                Tab::Largest => self.largest_table(ui, &mut actions),
                Tab::Repos => self.repos_table(ui, &mut actions),
                Tab::Volumes | Tab::Bin => {}
            }
        });
        for a in actions {
            self.apply(a, &ctx);
        }
        self.trash_dialog(&ctx);
        self.bin_dialog(&ctx);
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let scanning = self.scanning();
        let ctx = ui.ctx().clone();
        egui::Panel::top("top").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui
                        .add(
                            egui::Button::new("Open Folder…")
                                .shortcut_text(ctx.format_shortcut(&OPEN_KEY)),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.choose_folder();
                    }
                    if ui
                        .add_enabled(
                            !scanning && self.root_path.is_some(),
                            egui::Button::new("Rescan")
                                .shortcut_text(ctx.format_shortcut(&RESCAN_KEY)),
                        )
                        .clicked()
                    {
                        ui.close();
                        if let Some(p) = self.root_path.clone() {
                            self.start_scan(p);
                        }
                    }
                    if ui
                        .add_enabled(
                            scanning,
                            egui::Button::new("Stop Scan")
                                .shortcut_text(ctx.format_shortcut(&STOP_KEY)),
                        )
                        .clicked()
                    {
                        self.stop_scan();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        tray::request_quit();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    let has_sel = self.selected.iter().any(|&i| i != 0) && self.op.is_none();
                    if ui
                        .add(
                            egui::Button::new("Select All")
                                .shortcut_text(ctx.format_shortcut(&SELECT_ALL_KEY)),
                        )
                        .clicked()
                    {
                        self.selected = self.current_list().into_iter().collect();
                    }
                    if ui
                        .add_enabled(has_sel, egui::Button::new("Copy Paths"))
                        .clicked()
                    {
                        ui.close();
                        if let Some(t) = &self.tree {
                            let text: Vec<String> = self
                                .selected
                                .iter()
                                .map(|&i| t.path(i).display().to_string())
                                .collect();
                            ctx.copy_text(text.join("\n"));
                        }
                    }
                    ui.separator();
                    if ui
                        .add_enabled(has_sel, egui::Button::new("Move to…"))
                        .clicked()
                    {
                        ui.close();
                        self.ask_move();
                    }
                    if ui
                        .add_enabled(
                            has_sel,
                            egui::Button::new("Move to Trash…").shortcut_text("⌫"),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.confirm_trash = true;
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.show_inspector, "Inspector");
                    if ui
                        .checkbox(&mut self.folders_only, "Folders only")
                        .changed()
                    {
                        self.rows_dirty = true;
                    }
                    ui.separator();
                    ui.label("Sizes");
                    for (mode, label) in [
                        (SizeMode::OnDisk, "On disk"),
                        (SizeMode::Logical, "Logical"),
                    ] {
                        if ui.radio_value(&mut self.size_mode, mode, label).changed() {
                            self.size_mode_changed();
                        }
                    }
                    ui.separator();
                    ui.label("Theme");
                    for (key, label) in [("system", "System"), ("dark", "Dark"), ("light", "Light")]
                    {
                        if ui.radio_value(&mut self.theme, key, label).changed() {
                            apply_theme(ui.ctx(), key);
                        }
                    }
                });
                ui.menu_button("Help", |ui| {
                    ui.label(format!("RustDU {VERSION}"));
                    ui.weak("Drop a folder on the window to scan it.");
                    ui.weak("Click selects, ⌘/Ctrl-click adds, Shift-click selects a range.");
                    ui.weak("Double-click a folder to expand it, a file to reveal it.");
                    ui.weak("Right-click for Move, Trash and more.");
                });
            });
            ui.horizontal(|ui| {
                if ui
                    .button("Open Folder…")
                    .on_hover_text(ctx.format_shortcut(&OPEN_KEY))
                    .clicked()
                {
                    self.choose_folder();
                }
                if scanning {
                    if ui.button("Stop").clicked() {
                        self.stop_scan();
                    }
                } else if ui
                    .add_enabled(self.root_path.is_some(), egui::Button::new("Rescan"))
                    .on_hover_text(ctx.format_shortcut(&RESCAN_KEY))
                    .clicked()
                    && let Some(p) = self.root_path.clone()
                {
                    self.start_scan(p);
                }
                ui.separator();
                ui.selectable_value(&mut self.tab, Tab::Volumes, "Volumes");
                ui.add_enabled_ui(self.tree.is_some(), |ui| {
                    ui.selectable_value(&mut self.tab, Tab::Tree, "Explorer");
                    ui.selectable_value(&mut self.tab, Tab::Largest, "Largest files");
                    ui.selectable_value(
                        &mut self.tab,
                        Tab::Repos,
                        format!("Git repos ({})", self.repos.len()),
                    );
                });
                ui.selectable_value(
                    &mut self.tab,
                    Tab::Bin,
                    if cfg!(windows) {
                        "Recycle Bin"
                    } else {
                        "Trash"
                    },
                );
                ui.separator();
                egui::ComboBox::from_id_salt("size-mode")
                    .selected_text(match self.size_mode {
                        SizeMode::OnDisk => "On disk",
                        SizeMode::Logical => "Logical",
                    })
                    .show_ui(ui, |ui| {
                        let mut changed = false;
                        changed |= ui
                            .selectable_value(&mut self.size_mode, SizeMode::OnDisk, "On disk")
                            .changed();
                        changed |= ui
                            .selectable_value(&mut self.size_mode, SizeMode::Logical, "Logical")
                            .changed();
                        if changed {
                            self.size_mode_changed();
                        }
                    })
                    .response
                    .on_hover_text(Col::Size.tip());
                if self.tab == Tab::Tree
                    && ui
                        .checkbox(&mut self.folders_only, "Folders only")
                        .changed()
                {
                    self.rows_dirty = true;
                }
                ui.checkbox(&mut self.show_inspector, "Inspector");
                if self.busy() {
                    ui.spinner();
                }
                ui.add(egui::Label::new(egui::RichText::new(&self.message).weak()).truncate());
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(op) = &self.op {
                    ui.spinner();
                    ui.label(&op.label);
                    let n = op.files.load(Ordering::Relaxed);
                    if n > 0 {
                        ui.weak(format!("{} files copied", count(n as u64)));
                    }
                } else if self.scanning() {
                    let (items, bytes, current) = &self.progress_snapshot;
                    ui.spinner();
                    ui.monospace(format!(
                        "Scanning… {} items · {} · {:.1} s",
                        count(*items as u64),
                        size(*bytes),
                        self.elapsed
                    ));
                    ui.add(egui::Label::new(egui::RichText::new(current).weak()).truncate());
                } else if let Some(t) = &self.tree {
                    let r = t.root();
                    let when = if self.restored {
                        format!("scanned {}", ago(self.scanned_at, now()))
                    } else {
                        format!("scanned in {:.1} s", self.elapsed)
                    };
                    ui.monospace(format!(
                        "{} items · {} · {when}",
                        count(r.items()),
                        size(r.size(self.size_mode)),
                    ))
                    .on_hover_text(format!(
                        "Scanned {}. Rescan to bring it up to date.",
                        absolute(self.scanned_at)
                    ));
                    if self.cancelled {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "· stopped early, totals are incomplete",
                        );
                    }
                    if !self.repos.is_empty() {
                        let probing = self.git_done < self.repos.len();
                        ui.weak(if probing {
                            format!("· reading git {}/{}", self.git_done, self.repos.len())
                        } else {
                            format!("· {} git repos", self.repos.len())
                        });
                    }
                    if !scan::git_available() {
                        ui.weak("· git not found: repo details are approximate");
                    }
                    let n = self.selected.len();
                    if n > 0 {
                        let total: u64 = self
                            .selected
                            .iter()
                            .map(|&i| t.nodes[i as usize].size(self.size_mode))
                            .sum();
                        ui.separator();
                        ui.label(format!("{n} selected · {}", size(total)));
                    }
                } else {
                    ui.weak("Ready");
                }
            });
        });
    }

    fn header(&self, ui: &mut egui::Ui) {
        let Some(t) = &self.tree else { return };
        let r = t.root();
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(egui::RichText::new(t.root_path.display().to_string()).strong()).truncate());
            ui.separator();
            let stat = |ui: &mut egui::Ui, label: &str, value: String| {
                ui.weak(label);
                ui.monospace(value);
            };
            stat(ui, "Total", size(r.size(self.size_mode)));
            stat(ui, "Files", count(r.files));
            stat(ui, "Folders", count(r.dirs));
            stat(ui, "Last activity", ago(r.newest, now()));
            if r.unreadable > 0 {
                ui.colored_label(ui.visuals().error_fg_color, format!("🔒 {} unreadable", count(r.unreadable)))
                    .on_hover_text(if cfg!(target_os = "macos") {
                        "Some folders could not be read. Grant Full Disk Access in System Settings › Privacy & Security to include them."
                    } else {
                        "Some folders could not be read with your permissions."
                    });
            }
        });
        if let Some(v) = &self.root_volume {
            ui.horizontal(|ui| {
                ui.weak("Volume");
                ui.label(egui::RichText::new(&v.name).strong());
                capacity_bar(ui, v.used, v.total, Some(r.allocated), 260.0, 14.0);
                ui.monospace(format!("{} used · {} free of {}", size(v.used), size(v.free), size(v.total)));
                let elsewhere = v.used.saturating_sub(r.allocated);
                ui.weak(format!(
                    "· this scan {} ({} of used) · {} elsewhere on the volume",
                    size(r.allocated),
                    percent(r.allocated as f64 / v.used.max(1) as f64),
                    size(elsewhere)
                ))
                .on_hover_text(
                    "Space used outside the scanned folder: other users, system files, applications, caches, \
                     snapshots and purgeable space. Scan the volume from the Volumes tab to see it all.",
                );
            });
        }
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            if self.scanning() {
                let (items, bytes, current) = self.progress_snapshot.clone();
                ui.spinner();
                ui.heading(format!(
                    "Scanning {}…",
                    self.root_path
                        .as_ref()
                        .map_or(String::new(), |p| p.file_name().map_or_else(
                            || p.display().to_string(),
                            |n| n.to_string_lossy().into_owned()
                        ))
                ));
                ui.monospace(format!("{} items · {}", count(items as u64), size(bytes)));
                ui.weak(current);
                if ui.button("Stop").clicked() {
                    self.stop_scan();
                }
            } else {
                ui.heading("Scan a folder to see where the space is going");
                ui.weak("Drop a folder on the window, choose one, or pick a volume.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(ui.available_width() / 2.0 - 110.0);
                    if ui.button("Open Folder…").clicked() {
                        self.choose_folder();
                    }
                    if ui.button("Volumes").clicked() {
                        self.tab = Tab::Volumes;
                    }
                });
                if !self.message.is_empty() {
                    ui.add_space(8.0);
                    ui.colored_label(ui.visuals().error_fg_color, &self.message);
                }
            }
        });
    }

    // MARK: volumes

    fn volumes_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading("Volumes");
            ui.weak("used and free space per disk; Scan walks a whole volume");
            if ui.small_button("Refresh").clicked() {
                self.refresh_volumes(true);
            }
        });
        if cfg!(target_os = "macos") {
            ui.weak(
                "APFS volumes in one container share their free space. Time Machine snapshots, caches and \
                 other purgeable items count as used until macOS reclaims them. Your home folder is on the Data volume.",
            );
        }
        ui.add_space(6.0);
        if self.volumes.is_empty() {
            ui.weak("No volumes found.");
            return;
        }
        let scanned = self
            .tree
            .as_ref()
            .map(|t| (t.root_path.clone(), t.root().allocated));
        let busy = self.busy();
        let volumes = self.volumes.clone();
        TableBuilder::new(ui)
            .id_salt("volumes")
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(220.0).at_least(140.0).clip(true))
            .column(Column::initial(90.0).at_least(60.0))
            .column(Column::initial(380.0).at_least(240.0))
            .column(Column::initial(100.0).at_least(80.0))
            .column(Column::initial(100.0).at_least(80.0))
            .column(Column::initial(100.0).at_least(80.0))
            .column(Column::remainder().at_least(220.0).clip(true))
            .min_scrolled_height(0.0)
            .header(24.0, |mut h| {
                for label in ["Volume", "Type", "Used", "Total", "Free", "", "Mount point"] {
                    h.col(|ui| {
                        ui.label(egui::RichText::new(label).strong());
                    });
                }
            })
            .body(|mut body| {
                for v in &volumes {
                    let scan_here = scanned
                        .as_ref()
                        .filter(|(p, _)| {
                            self.root_volume
                                .as_ref()
                                .is_some_and(|rv| rv.mount == v.mount)
                                && p.starts_with(&v.mount)
                        })
                        .map(|(_, bytes)| *bytes);
                    body.row(34.0, |mut row| {
                        row.col(|ui| {
                            ui.label(egui::RichText::new("💽").color(folder_color(ui)));
                            ui.add(
                                egui::Label::new(egui::RichText::new(&v.name).strong()).truncate(),
                            );
                        });
                        row.col(|ui| {
                            ui.weak(&v.fs);
                        });
                        row.col(|ui| {
                            capacity_bar(ui, v.used, v.total, scan_here, 300.0, 16.0);
                        });
                        row.col(|ui| {
                            ui.monospace(size(v.total));
                        });
                        row.col(|ui| {
                            ui.monospace(size(v.free));
                        });
                        row.col(|ui| {
                            if ui
                                .add_enabled(!busy, egui::Button::new("Scan"))
                                .on_hover_text("Scan this whole volume")
                                .clicked()
                            {
                                actions.push(Action::ScanPath(v.mount.clone()));
                            }
                        });
                        row.col(|ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(v.mount.display().to_string()).weak(),
                                )
                                .truncate(),
                            );
                            if let Some(bytes) = scan_here {
                                badge(ui, &format!("scanned {}", size(bytes)), PURPLE);
                            }
                        });
                    });
                }
            });
        ui.add_space(8.0);
        ui.weak("Used = everything on the volume. The purple part of a bar is what the current scan covers.");
    }

    // MARK: tables

    fn tree_table(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        const COLS: [Col; 9] = [
            Col::Name,
            Col::Size,
            Col::Percent,
            Col::Items,
            Col::Activity,
            Col::Modified,
            Col::Perms,
            Col::Owner,
            Col::Git,
        ];
        let now = now();
        let mode = self.size_mode;
        let sort = self.sort;
        let sel_count = self.selected.len();
        let mut header_clicks: Vec<Col> = Vec::new();
        let table = TableBuilder::new(ui)
            .id_salt("tree")
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(360.0).at_least(180.0).clip(true))
            .column(Column::initial(150.0).at_least(120.0))
            .column(Column::initial(80.0).at_least(60.0))
            .column(Column::initial(80.0).at_least(60.0))
            .column(Column::initial(110.0).at_least(90.0))
            .column(Column::initial(110.0).at_least(90.0))
            .column(Column::initial(100.0).at_least(80.0))
            .column(Column::initial(120.0).at_least(80.0).clip(true))
            .column(Column::remainder().at_least(160.0).clip(true))
            .min_scrolled_height(0.0)
            .header(24.0, |mut h| {
                for col in COLS {
                    h.col(|ui| {
                        if header_button(ui, col, sort).clicked() {
                            header_clicks.push(col);
                        }
                    });
                }
            });
        let Self {
            tree,
            rows,
            expanded,
            owners,
            selected,
            ..
        } = self;
        let Some(tree) = tree.as_ref() else { return };
        table.body(|body| {
            body.rows(ROW_HEIGHT, rows.len(), |mut row| {
                let (idx, depth) = rows[row.index()];
                let n = &tree.nodes[idx as usize];
                let in_sel = selected.contains(&idx);
                row.set_selected(in_sel);
                row.col(|ui| {
                    ui.add_space(depth as f32 * 16.0);
                    if n.is_dir() && !n.is_skipped {
                        let arrow = if expanded.contains(&idx) {
                            "▾"
                        } else {
                            "▸"
                        };
                        if ui
                            .add(egui::Button::new(arrow).frame(false).small())
                            .clicked()
                        {
                            actions.push(Action::Toggle(idx));
                        }
                    } else {
                        ui.add_space(22.0);
                    }
                    name_cell(ui, tree, idx, now);
                });
                row.col(|ui| size_cell(ui, n.size(mode), tree.percent_of_parent(idx, mode), true));
                row.col(|ui| {
                    ui.weak(percent(tree.percent_of_parent(idx, mode)));
                });
                row.col(|ui| {
                    if n.is_dir() {
                        ui.weak(count(n.items()));
                    }
                });
                row.col(|ui| date_cell(ui, n.newest, now));
                row.col(|ui| date_cell(ui, n.modified, now));
                row.col(|ui| perm_cell(ui, tree, idx));
                row.col(|ui| {
                    let owner = owners
                        .entry(idx)
                        .or_insert_with(|| scan::owner(&tree.path(idx), n.uid, n.gid));
                    ui.add(egui::Label::new(egui::RichText::new(owner.as_str()).weak()).truncate());
                });
                row.col(|ui| git_cell(ui, n, now));
                row_interactions(&row, idx, n.is_dir(), in_sel, sel_count, actions);
            });
        });
        for col in header_clicks {
            self.click_sort(Tab::Tree, col);
        }
    }

    fn largest_table(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        const COLS: [Col; 6] = [
            Col::Name,
            Col::Size,
            Col::Modified,
            Col::Perms,
            Col::Owner,
            Col::Path,
        ];
        let now = now();
        let mode = self.size_mode;
        let sort = self.sort_largest;
        let sel_count = self.selected.len();
        let Self {
            tree,
            largest,
            owners,
            selected,
            ..
        } = self;
        let Some(tree) = tree.as_ref() else { return };
        let total: u64 = largest
            .iter()
            .map(|&i| tree.nodes[i as usize].size(mode))
            .sum();
        let root_size = tree.root().size(mode).max(1);
        ui.horizontal(|ui| {
            ui.weak(format!("The {} largest files", largest.len()));
            ui.separator();
            ui.weak(format!(
                "together {} · {} of the scan",
                size(total),
                percent(total as f64 / root_size as f64)
            ));
        });
        let mut header_clicks: Vec<Col> = Vec::new();
        let root_path = tree.root_path.clone();
        TableBuilder::new(ui)
            .id_salt("largest")
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(300.0).at_least(160.0).clip(true))
            .column(Column::initial(100.0).at_least(80.0))
            .column(Column::initial(110.0).at_least(90.0))
            .column(Column::initial(100.0).at_least(80.0))
            .column(Column::initial(120.0).at_least(80.0).clip(true))
            .column(Column::remainder().at_least(200.0).clip(true))
            .min_scrolled_height(0.0)
            .header(24.0, |mut h| {
                for col in COLS {
                    h.col(|ui| {
                        if header_button(ui, col, sort).clicked() {
                            header_clicks.push(col);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, largest.len(), |mut row| {
                    let idx = largest[row.index()];
                    let n = &tree.nodes[idx as usize];
                    let in_sel = selected.contains(&idx);
                    row.set_selected(in_sel);
                    row.col(|ui| name_cell(ui, tree, idx, now));
                    row.col(|ui| size_cell(ui, n.size(mode), 0.0, false));
                    row.col(|ui| date_cell(ui, n.modified, now));
                    row.col(|ui| perm_cell(ui, tree, idx));
                    row.col(|ui| {
                        let owner = owners
                            .entry(idx)
                            .or_insert_with(|| scan::owner(&tree.path(idx), n.uid, n.gid));
                        ui.add(
                            egui::Label::new(egui::RichText::new(owner.as_str()).weak()).truncate(),
                        );
                    });
                    row.col(|ui| {
                        let p = tree.path(idx);
                        let folder = p.parent().unwrap_or(&p);
                        let rel = folder.strip_prefix(&root_path).unwrap_or(folder);
                        let text = if rel.as_os_str().is_empty() {
                            ".".to_string()
                        } else {
                            format!(".{}{}", std::path::MAIN_SEPARATOR, rel.display())
                        };
                        ui.add(egui::Label::new(egui::RichText::new(text).weak()).truncate())
                            .on_hover_text(p.display().to_string());
                    });
                    row_interactions(&row, idx, false, in_sel, sel_count, actions);
                });
            });
        for col in header_clicks {
            self.click_sort(Tab::Largest, col);
        }
    }

    fn repos_table(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        const COLS: [Col; 9] = [
            Col::Name,
            Col::Branch,
            Col::Git,
            Col::Changes,
            Col::Size,
            Col::GitDir,
            Col::Activity,
            Col::Remote,
            Col::Path,
        ];
        let now = now();
        let mode = self.size_mode;
        let sort = self.sort_repos;
        let sel_count = self.selected.len();
        let Self {
            tree,
            repos,
            selected,
            ..
        } = self;
        let Some(tree) = tree.as_ref() else { return };
        if repos.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.heading("No git repositories found");
                ui.weak("Folders that contain a .git entry show up here.");
            });
            return;
        }
        let total: u64 = repos
            .iter()
            .map(|&i| tree.nodes[i as usize].size(mode))
            .sum();
        let history: u64 = repos
            .iter()
            .filter_map(|&i| tree.git_dir(i))
            .map(|d| tree.nodes[d as usize].size(mode))
            .sum();
        ui.horizontal(|ui| {
            ui.weak(format!("{} repositories", repos.len()));
            ui.separator();
            ui.weak(format!(
                "repos {} · of which .git history {}",
                size(total),
                size(history)
            ));
        });
        let mut header_clicks: Vec<Col> = Vec::new();
        TableBuilder::new(ui)
            .id_salt("repos")
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(220.0).at_least(140.0).clip(true))
            .column(Column::initial(140.0).at_least(90.0).clip(true))
            .column(Column::initial(120.0).at_least(90.0))
            .column(Column::initial(80.0).at_least(60.0))
            .column(Column::initial(90.0).at_least(70.0))
            .column(Column::initial(90.0).at_least(70.0))
            .column(Column::initial(110.0).at_least(90.0))
            .column(Column::initial(260.0).at_least(120.0).clip(true))
            .column(Column::remainder().at_least(160.0).clip(true))
            .min_scrolled_height(0.0)
            .header(24.0, |mut h| {
                for col in COLS {
                    h.col(|ui| {
                        let label = if col == Col::Git {
                            "Last commit"
                        } else {
                            col.label()
                        };
                        let text = if sort.col == col {
                            format!("{label} {}", if sort.desc { "▼" } else { "▲" })
                        } else {
                            label.to_string()
                        };
                        if ui
                            .add(egui::Button::new(egui::RichText::new(text).strong()).frame(false))
                            .clicked()
                        {
                            header_clicks.push(col);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, repos.len(), |mut row| {
                    let idx = repos[row.index()];
                    let n = &tree.nodes[idx as usize];
                    let in_sel = selected.contains(&idx);
                    row.set_selected(in_sel);
                    row.col(|ui| {
                        ui.label(egui::RichText::new("📁").color(folder_color(ui)));
                        ui.add(egui::Label::new(&n.name).truncate());
                    });
                    row.col(|ui| match &n.git {
                        Some(g) => {
                            ui.add(egui::Label::new(&g.branch).truncate());
                            if g.detached {
                                badge(ui, "detached", ORANGE);
                            }
                        }
                        None => {
                            ui.spinner();
                        }
                    });
                    row.col(|ui| match n.git.as_ref().and_then(|g| g.last_commit) {
                        Some(d) => date_cell(ui, d, now),
                        None => {
                            ui.weak("—");
                        }
                    });
                    row.col(|ui| {
                        if let Some(g) = &n.git {
                            match g.changed {
                                Some(0) => {
                                    ui.colored_label(GREEN, "clean");
                                }
                                Some(c) => {
                                    ui.colored_label(ORANGE, format!("● {c}"))
                                        .on_hover_text("Changed or untracked files");
                                }
                                None => {
                                    ui.weak("?");
                                }
                            }
                        }
                    });
                    row.col(|ui| {
                        ui.monospace(size(n.size(mode)));
                    });
                    row.col(|ui| {
                        let s = tree.git_dir(idx).map_or("—".to_string(), |d| {
                            size(tree.nodes[d as usize].size(mode))
                        });
                        ui.weak(egui::RichText::new(s).monospace());
                    });
                    row.col(|ui| date_cell(ui, n.newest, now));
                    row.col(|ui| {
                        let remote = n.git.as_ref().map(|g| g.remote.as_str()).unwrap_or("");
                        ui.add(egui::Label::new(egui::RichText::new(remote).weak()).truncate());
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(tree.path(idx).display().to_string()).weak(),
                            )
                            .truncate(),
                        );
                    });
                    row_interactions(&row, idx, true, in_sel, sel_count, actions);
                });
            });
        for col in header_clicks {
            self.click_sort(Tab::Repos, col);
        }
    }

    // MARK: inspector

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let mut action: Option<Action> = None;
        let mode = self.size_mode;
        let now = now();
        let multi = self.selected.len() > 1;
        let idx = self
            .anchor
            .filter(|a| self.selected.contains(a))
            .or_else(|| self.selected.iter().next().copied())
            .unwrap_or(0);
        let owner = if multi {
            String::new()
        } else {
            self.owner_of(idx)
        };
        let busy = self.op.is_some();
        let Some(tree) = &self.tree else { return };
        let Some(n) = tree.nodes.get(idx as usize) else {
            return;
        };
        let path = tree.path(idx);
        let selected = &self.selected;
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(360.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(4.0);
                    if multi {
                        let (mut disk, mut logical, mut files, mut dirs) = (0u64, 0u64, 0u64, 0u64);
                        for &i in selected {
                            let s = &tree.nodes[i as usize];
                            disk += s.allocated;
                            logical += s.logical;
                            if s.is_dir() {
                                files += s.files;
                                dirs += 1 + s.dirs;
                            } else {
                                files += 1;
                            }
                        }
                        ui.heading(format!("{} items selected", selected.len()));
                        egui::Grid::new("multi").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                            kv(ui, "On disk", size(disk));
                            kv(ui, "Logical", size(logical));
                            kv(ui, "Files", count(files));
                            kv(ui, "Folders", count(dirs));
                        });
                        ui.add_space(6.0);
                        section(ui, "Selected");
                        for &i in selected.iter().take(30) {
                            ui.add(egui::Label::new(egui::RichText::new(&tree.nodes[i as usize].name).weak()).truncate());
                        }
                        if selected.len() > 30 {
                            ui.weak(format!("… and {} more", selected.len() - 30));
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(icon_for(n)).heading().color(if n.is_dir() {
                                folder_color(ui)
                            } else {
                                ui.visuals().weak_text_color()
                            }));
                            ui.add(egui::Label::new(egui::RichText::new(&n.name).heading()).truncate());
                        });
                        ui.add(egui::Label::new(egui::RichText::new(path.display().to_string()).weak()).wrap());
                        ui.label(kind_text(n));
                        if n.is_skipped {
                            ui.weak("Another volume or pseudo filesystem: not scanned, scan it from the Volumes tab.");
                        }
                        ui.add_space(6.0);

                        section(ui, "Size");
                        egui::Grid::new("size").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                            kv(ui, "On disk", size(n.allocated));
                            kv(ui, "Logical", size(n.logical));
                            if n.logical > n.allocated && n.allocated > 0 {
                                kv(ui, "Saved by compression", size(n.logical - n.allocated));
                            }
                            if idx != 0 {
                                kv(ui, "Of parent", percent(tree.percent_of_parent(idx, mode)));
                                let root = tree.root().size(mode).max(1);
                                kv(ui, "Of scan", percent(n.size(mode) as f64 / root as f64));
                            }
                            if n.is_dir() {
                                kv(ui, "Files", count(n.files));
                                kv(ui, "Folders", count(n.dirs));
                                if n.unreadable > 0 {
                                    ui.label("Unreadable");
                                    ui.colored_label(ui.visuals().error_fg_color, count(n.unreadable));
                                    ui.end_row();
                                }
                            }
                        });
                        if n.hard_dup {
                            ui.weak("Hard link to a file counted elsewhere; counted as 0 here.");
                        }
                        ui.add_space(6.0);

                        section(ui, "Dates");
                        egui::Grid::new("dates").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                            kv(ui, "Modified", format!("{}  ({})", absolute(n.modified), ago(n.modified, now)));
                            if n.is_dir() {
                                kv(
                                    ui,
                                    "Last activity inside",
                                    format!("{}  ({})", absolute(n.newest), ago(n.newest, now)),
                                );
                            }
                        });
                        if n.is_dir() && now - n.newest > STALE_SECS {
                            ui.weak("💤 Nothing in here changed in over a year.");
                        }
                        ui.add_space(6.0);

                        section(ui, "Permissions");
                        egui::Grid::new("perms").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                            let perms = scan::perm_string(n.kind, n.mode, n.attrs);
                            if cfg!(windows) {
                                kv(ui, "Attributes", perms);
                                kv(ui, "Owner", owner.clone());
                            } else {
                                ui.label("Mode");
                                ui.monospace(format!("{perms}  {}", scan::octal_string(n.mode)));
                                ui.end_row();
                                let (u, g) = owner.split_once(':').unwrap_or((owner.as_str(), ""));
                                kv(ui, "Owner", format!("{u} ({})", n.uid));
                                kv(ui, "Group", format!("{g} ({})", n.gid));
                            }
                            kv(ui, "You can write", if scan::can_write(&path) { "Yes".into() } else { "No".into() });
                        });
                        ui.add_space(6.0);

                        if n.is_git {
                            section(ui, "Git repository");
                            egui::Grid::new("git").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                                match &n.git {
                                    Some(g) => {
                                        kv(
                                            ui,
                                            "Branch",
                                            format!("{}{}", g.branch, if g.detached { " (detached)" } else { "" }),
                                        );
                                        if let Some(d) = g.last_commit {
                                            kv(ui, "Last commit", format!("{}  ({})", absolute(d), ago(d, now)));
                                        }
                                        if !g.subject.is_empty() {
                                            ui.label("Message");
                                            ui.add(egui::Label::new(&g.subject).wrap());
                                            ui.end_row();
                                        }
                                        let wt = match (g.fallback, g.changed) {
                                            (true, _) | (false, None) => "unknown (git not available)".to_string(),
                                            (false, Some(0)) => "Clean".into(),
                                            (false, Some(c)) => format!("{c} changed or untracked"),
                                        };
                                        kv(ui, "Working tree", wt);
                                        if !g.remote.is_empty() {
                                            ui.label("Remote");
                                            ui.add(egui::Label::new(&g.remote).wrap());
                                            ui.end_row();
                                        }
                                    }
                                    None => {
                                        ui.label("Reading…");
                                        ui.spinner();
                                        ui.end_row();
                                    }
                                }
                                if let Some(d) = tree.git_dir(idx) {
                                    kv(ui, ".git folder", size(tree.nodes[d as usize].size(mode)));
                                }
                            });
                            ui.add_space(6.0);
                        }
                    }

                    section(ui, "Actions");
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Reveal").on_hover_text("Show in Finder / Explorer").clicked() {
                            action = Some(Action::Reveal(idx));
                        }
                        if ui.button(if multi { "Copy paths" } else { "Copy path" }).clicked() {
                            action = Some(Action::CopyPaths);
                        }
                        if !multi && ui.button("Open terminal").clicked() {
                            action = Some(Action::Terminal(idx));
                        }
                        if !multi && n.is_dir() && idx != 0 && ui.button("Scan this folder").clicked() {
                            action = Some(Action::ScanFolder(idx));
                        }
                    });
                    let can_change = !busy && selected.iter().any(|&i| i != 0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(can_change, egui::Button::new("Move to…")).clicked() {
                            action = Some(Action::AskMove);
                        }
                        if ui.add_enabled(can_change, egui::Button::new("Move to Trash…")).clicked() {
                            action = Some(Action::AskTrash);
                        }
                    });
                });
            });
        if let Some(a) = action {
            if matches!(a, Action::CopyPaths) && selected.is_empty() {
                ui.ctx().copy_text(path.display().to_string());
            } else {
                let ctx = ui.ctx().clone();
                self.apply(a, &ctx);
            }
        }
    }

    fn owner_of(&mut self, idx: u32) -> String {
        let Some(tree) = &self.tree else {
            return String::new();
        };
        self.owners
            .entry(idx)
            .or_insert_with(|| {
                let n = &tree.nodes[idx as usize];
                scan::owner(&tree.path(idx), n.uid, n.gid)
            })
            .clone()
    }

    fn trash_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_trash {
            return;
        }
        let targets = self.operation_targets();
        let Some(tree) = &self.tree else {
            self.confirm_trash = false;
            return;
        };
        if targets.is_empty() {
            self.confirm_trash = false;
            return;
        }
        let total: u64 = targets
            .iter()
            .map(|(i, _)| tree.nodes[*i as usize].size(self.size_mode))
            .sum();
        let items: u64 = targets
            .iter()
            .map(|(i, _)| {
                let n = &tree.nodes[*i as usize];
                if n.is_dir() { n.items() } else { 1 }
            })
            .sum();
        let title = if targets.len() == 1 {
            format!(
                "Move “{}” to the Trash?",
                tree.nodes[targets[0].0 as usize].name
            )
        } else {
            format!("Move {} items to the Trash?", targets.len())
        };
        let mut decision: Option<OpKind> = None;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::unique("trash")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.heading(title);
            ui.label(format!("{} · {} items", size(total), count(items)));
            for (_, p) in targets.iter().take(8) {
                ui.add(
                    egui::Label::new(egui::RichText::new(p.display().to_string()).weak())
                        .truncate(),
                );
            }
            if targets.len() > 8 {
                ui.weak(format!("… and {} more", targets.len() - 8));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                let trash = egui::Button::new(
                    egui::RichText::new("Move to Trash").color(egui::Color32::WHITE),
                )
                .fill(egui::Color32::from_rgb(0xC9, 0x3C, 0x3C));
                if ui.add(trash).clicked() {
                    decision = Some(OpKind::Trash);
                }
                if ui
                    .button("Delete permanently")
                    .on_hover_text("Skips the Trash: faster for huge folders, cannot be undone")
                    .clicked()
                {
                    decision = Some(OpKind::Delete);
                }
            });
        });
        if modal.should_close() {
            cancel = true;
        }
        if let Some(kind) = decision {
            self.start_op(kind);
        } else if cancel {
            self.confirm_trash = false;
        }
    }
}

// MARK: cells

fn header_button(ui: &mut egui::Ui, col: Col, sort: Sort) -> egui::Response {
    let text = if sort.col == col {
        format!("{} {}", col.label(), if sort.desc { "▼" } else { "▲" })
    } else {
        col.label().to_string()
    };
    let r = ui.add(egui::Button::new(egui::RichText::new(text).strong()).frame(false));
    if col.tip().is_empty() {
        r
    } else {
        r.on_hover_text(col.tip())
    }
}

fn row_interactions(
    row: &TableRow<'_, '_>,
    idx: u32,
    is_dir: bool,
    in_sel: bool,
    sel_count: usize,
    actions: &mut Vec<Action>,
) {
    let r = row.response();
    let m = r.ctx.input(|i| i.modifiers);
    if r.clicked() {
        actions.push(Action::Select {
            idx,
            toggle: m.command,
            range: m.shift,
        });
    }
    // Right-click on something outside the selection makes it the selection.
    if r.secondary_clicked() && !in_sel {
        actions.push(Action::Select {
            idx,
            toggle: false,
            range: false,
        });
    }
    if r.double_clicked() {
        actions.push(if is_dir {
            Action::Toggle(idx)
        } else {
            Action::Reveal(idx)
        });
    }
    let n = if in_sel { sel_count.max(1) } else { 1 };
    let what = if n == 1 {
        String::new()
    } else {
        format!(" {n} items")
    };
    r.context_menu(|ui| {
        if ui.button("Reveal in file manager").clicked() {
            actions.push(Action::Reveal(idx));
            ui.close();
        }
        if ui
            .button(if n == 1 { "Copy path" } else { "Copy paths" })
            .clicked()
        {
            actions.push(Action::CopyPaths);
            ui.close();
        }
        if ui.button("Open terminal here").clicked() {
            actions.push(Action::Terminal(idx));
            ui.close();
        }
        if is_dir && n == 1 {
            if ui.button("Scan this folder").clicked() {
                actions.push(Action::ScanFolder(idx));
                ui.close();
            }
            if ui.button("Expand / collapse").clicked() {
                actions.push(Action::Toggle(idx));
                ui.close();
            }
        }
        ui.separator();
        if ui.button(format!("Move{what} to…")).clicked() {
            actions.push(Action::AskMove);
            ui.close();
        }
        if ui.button(format!("Move{what} to Trash…")).clicked() {
            actions.push(Action::AskTrash);
            ui.close();
        }
    });
}

fn folder_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(0x5B, 0xA8, 0xF5)
    } else {
        egui::Color32::from_rgb(0x1E, 0x6F, 0xD9)
    }
}

fn icon_for(n: &scan::Node) -> &'static str {
    match n.kind {
        Kind::Dir => "📁",
        Kind::File => "📄",
        Kind::Symlink => "🔗",
        Kind::Other => "▫",
    }
}

fn kind_text(n: &scan::Node) -> &'static str {
    match n.kind {
        Kind::Dir if n.is_git => "Folder (git repository)",
        Kind::Dir => "Folder",
        Kind::File => "File",
        Kind::Symlink => "Symbolic link",
        Kind::Other => "Special file",
    }
}

fn name_cell(ui: &mut egui::Ui, tree: &Tree, idx: u32, now: i64) {
    let n = &tree.nodes[idx as usize];
    let stale = n.is_dir() && !n.is_skipped && now - n.newest > STALE_SECS;
    let icon_color = match n.kind {
        Kind::Dir if stale || n.is_skipped => ui.visuals().weak_text_color(),
        Kind::Dir => folder_color(ui),
        _ => ui.visuals().weak_text_color(),
    };
    ui.label(egui::RichText::new(icon_for(n)).color(icon_color));
    let mut text = egui::RichText::new(&n.name);
    if stale || n.is_skipped {
        text = text.weak();
    }
    ui.add(egui::Label::new(text).truncate());
    if n.is_git {
        let label = n.git.as_ref().map_or("git", |g| g.branch.as_str());
        badge(ui, label, PURPLE);
    }
    if n.is_unreadable {
        ui.label("🔒")
            .on_hover_text("Not readable with your permissions");
    }
    if n.is_skipped {
        badge(ui, "other volumes", ui.visuals().weak_text_color());
    }
    if n.hard_dup {
        badge(ui, "hard link", ui.visuals().weak_text_color());
    }
    if stale {
        ui.weak("💤")
            .on_hover_text("Nothing in here changed in the last year");
    }
}

fn badge(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.2))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).small().color(color));
        });
}

/// Level colour: green while there is room, orange from 80%, red from 92%.
fn level_color(fraction: f64) -> egui::Color32 {
    if fraction >= 0.92 {
        RED
    } else if fraction >= 0.80 {
        ORANGE
    } else {
        GREEN
    }
}

/// A capacity indicator: rounded track, filled by `used / total` in a level colour, with the
/// optional `highlight` bytes (the current scan) drawn as a purple segment at the start and
/// the percentage printed on the bar.
fn capacity_bar(
    ui: &mut egui::Ui,
    used: u64,
    total: u64,
    highlight: Option<u64>,
    width: f32,
    height: f32,
) {
    let frac = if total == 0 {
        0.0
    } else {
        used as f64 / total as f64
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter();
    let radius = egui::CornerRadius::same((height / 2.0) as u8);
    let track = if ui.visuals().dark_mode {
        egui::Color32::from_gray(0x3A)
    } else {
        egui::Color32::from_gray(0xD8)
    };
    painter.rect_filled(rect, radius, track);
    let fill_w = (rect.width() * frac.clamp(0.0, 1.0) as f32).max(if used > 0 { 3.0 } else { 0.0 });
    let fill = egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height()));
    painter.rect_filled(fill, radius, level_color(frac));
    if let Some(h) = highlight.filter(|&h| h > 0 && total > 0) {
        let hw = (rect.width() * (h as f64 / total as f64).clamp(0.0, 1.0) as f32).max(3.0);
        let hr = egui::Rect::from_min_size(rect.min, egui::vec2(hw.min(fill_w), rect.height()));
        painter.rect_filled(hr, radius, PURPLE);
    }
    // Tick marks every 10% give it the segmented look of a gauge.
    for i in 1..10 {
        let x = rect.min.x + rect.width() * i as f32 / 10.0;
        painter.line_segment(
            [
                egui::pos2(x, rect.min.y + 2.0),
                egui::pos2(x, rect.max.y - 2.0),
            ],
            egui::Stroke::new(1.0, track.gamma_multiply(1.6)),
        );
    }
    let label = format!("{:.0}%", frac * 100.0);
    let font = egui::FontId::proportional((height - 4.0).max(9.0));
    let text_color = egui::Color32::WHITE;
    painter.text(
        egui::pos2(rect.max.x - 6.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        &label,
        font.clone(),
        if frac > 0.85 {
            text_color
        } else {
            ui.visuals().text_color()
        },
    );
    if frac >= 0.12 {
        painter.text(
            egui::pos2(rect.min.x + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            size(used),
            font,
            text_color,
        );
    }
    resp.on_hover_text(match highlight {
        Some(h) if h > 0 => format!(
            "{} used of {} ({})\nof which this scan: {}",
            size(used),
            size(total),
            label,
            size(h)
        ),
        _ => format!("{} used of {} ({label})", size(used), size(total)),
    });
}

fn size_cell(ui: &mut egui::Ui, bytes: u64, fraction: f64, bar: bool) {
    if bar {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(70.0, 7.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(3),
            ui.visuals().faint_bg_color.gamma_multiply(1.5),
        );
        let color = if fraction >= 0.5 {
            RED
        } else if fraction >= 0.2 {
            ORANGE
        } else {
            folder_color(ui)
        };
        let w = (rect.width() * fraction.clamp(0.0, 1.0) as f32).max(2.0);
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(w, rect.height())),
            egui::CornerRadius::same(3),
            color,
        );
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.monospace(size(bytes))
            .on_hover_text(format!("{} bytes", count(bytes)));
    });
}

fn date_cell(ui: &mut egui::Ui, secs: i64, now: i64) {
    let text = ago(secs, now);
    let r = if now - secs > STALE_SECS {
        ui.weak(text)
    } else {
        ui.label(text)
    };
    r.on_hover_text(absolute(secs));
}

fn perm_cell(ui: &mut egui::Ui, tree: &Tree, idx: u32) {
    let n = &tree.nodes[idx as usize];
    let text = egui::RichText::new(scan::perm_string(n.kind, n.mode, n.attrs)).monospace();
    ui.label(text).on_hover_text(if cfg!(windows) {
        Col::Perms.tip().to_string()
    } else {
        format!("Octal {}", scan::octal_string(n.mode))
    });
}

fn git_cell(ui: &mut egui::Ui, n: &scan::Node, now: i64) {
    if !n.is_git {
        return;
    }
    match &n.git {
        Some(g) => {
            ui.add(egui::Label::new(&g.branch).truncate());
            match g.changed {
                Some(0) if !g.fallback => {
                    ui.colored_label(GREEN, "✓")
                        .on_hover_text("Working tree clean");
                }
                Some(c) if c > 0 => {
                    ui.colored_label(ORANGE, format!("● {c}"))
                        .on_hover_text("Changed or untracked files");
                }
                _ => {}
            }
            if let Some(d) = g.last_commit {
                ui.weak(ago(d, now)).on_hover_text(format!(
                    "Last commit {}\n{}",
                    absolute(d),
                    g.subject
                ));
            }
        }
        None => {
            ui.spinner();
            ui.weak("reading…");
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).strong());
}

fn kv(ui: &mut egui::Ui, key: &str, value: String) {
    ui.label(key);
    ui.monospace(value);
    ui.end_row();
}

// MARK: helpers

fn apply_theme(ctx: &egui::Context, theme: &str) {
    use egui::{Theme, ThemePreference, Visuals};
    ctx.set_visuals_of(Theme::Dark, Visuals::dark());
    ctx.set_visuals_of(Theme::Light, Visuals::light());
    match theme {
        "dark" => ctx.set_theme(ThemePreference::Dark),
        "light" => ctx.set_theme(ThemePreference::Light),
        _ => ctx.set_theme(ThemePreference::System),
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// "812 MB", "1.4 GB": decimal units like Finder and Explorer.
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e12 {
        format!("{:.2} TB", b / 1e12)
    } else if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} B")
    }
}

/// 1234567 → "1,234,567".
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn percent(f: f64) -> String {
    let p = f * 100.0;
    if p < 10.0 {
        format!("{p:.1}%")
    } else {
        format!("{p:.0}%")
    }
}

/// "3 days ago", "2 years ago"; "—" for an unknown time.
pub fn ago(secs: i64, now: i64) -> String {
    if secs <= 0 {
        return "—".into();
    }
    let d = now - secs;
    let plural = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };
    match d {
        i64::MIN..=59 => "just now".into(),
        60..=3599 => plural(d / 60, "minute"),
        3600..=86_399 => plural(d / 3600, "hour"),
        86_400..=172_799 => "yesterday".into(),
        172_800..=2_591_999 => plural(d / 86_400, "day"),
        2_592_000..=31_535_999 => plural(d / 2_592_000, "month"),
        _ => plural(d / 31_536_000, "year"),
    }
}

/// Local date and time, "2026-11-01 16:45".
pub fn absolute(secs: i64) -> String {
    use chrono::TimeZone;
    if secs <= 0 {
        return "—".into();
    }
    chrono::Local
        .timestamp_opt(secs, 0)
        .single()
        .map_or_else(|| "—".into(), |d| d.format("%Y-%m-%d %H:%M").to_string())
}

/// Select the item in Finder / Explorer, or open its folder elsewhere.
fn reveal(path: &Path) {
    let spawned = if cfg!(target_os = "macos") {
        scan::quiet_cmd("open").arg("-R").arg(path).spawn()
    } else if cfg!(windows) {
        scan::quiet_cmd("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
    } else {
        scan::quiet_cmd("xdg-open")
            .arg(path.parent().unwrap_or(path))
            .spawn()
    };
    let _ = spawned;
}

fn open_terminal(dir: &Path) {
    let spawned = if cfg!(target_os = "macos") {
        scan::quiet_cmd("open")
            .args(["-a", "Terminal"])
            .arg(dir)
            .spawn()
    } else if cfg!(windows) {
        scan::quiet_cmd("cmd")
            .args(["/C", "start", "", "cmd", "/K", "cd", "/d"])
            .arg(dir)
            .spawn()
    } else {
        scan::quiet_cmd("x-terminal-emulator")
            .current_dir(dir)
            .spawn()
    };
    let _ = spawned;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_decimal_units() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(999), "999 B");
        assert_eq!(size(1_500), "2 KB");
        assert_eq!(size(812_000_000), "812 MB");
        assert_eq!(size(1_400_000_000), "1.4 GB");
        assert_eq!(size(2_500_000_000_000), "2.50 TB");
    }

    #[test]
    fn counts_get_separators() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1000), "1,000");
        assert_eq!(count(1_234_567), "1,234,567");
    }

    #[test]
    fn relative_dates() {
        let now = 1_700_000_000;
        assert_eq!(ago(0, now), "—");
        assert_eq!(ago(now - 5, now), "just now");
        assert_eq!(ago(now - 120, now), "2 minutes ago");
        assert_eq!(ago(now - 3600, now), "1 hour ago");
        assert_eq!(ago(now - 90_000, now), "yesterday");
        assert_eq!(ago(now - 3 * 86_400, now), "3 days ago");
        assert_eq!(ago(now - 40 * 86_400, now), "1 month ago");
        assert_eq!(ago(now - 800 * 86_400, now), "2 years ago");
    }

    #[test]
    fn percent_keeps_a_decimal_under_ten() {
        assert_eq!(percent(0.0123), "1.2%");
        assert_eq!(percent(0.43), "43%");
    }

    #[test]
    fn level_colours_turn_orange_then_red() {
        assert_eq!(level_color(0.5), GREEN);
        assert_eq!(level_color(0.85), ORANGE);
        assert_eq!(level_color(0.95), RED);
    }
}
