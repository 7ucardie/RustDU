//! Walks a folder into a tree of nodes with sizes, dates, permissions and git state.
//! Runs on a background thread; the GUI polls `Progress` and receives the finished `Tree`.

use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Dir,
    File,
    Symlink,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SizeMode {
    /// Allocated blocks: what deleting the item frees.
    OnDisk,
    /// Byte length of the files.
    Logical,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GitInfo {
    pub branch: String,
    pub detached: bool,
    /// Unix seconds of the last commit.
    pub last_commit: Option<i64>,
    pub subject: String,
    /// Changed plus untracked files; None when git could not tell.
    pub changed: Option<usize>,
    pub remote: String,
    /// Read from .git/HEAD because git is not installed.
    pub fallback: bool,
}

#[derive(Debug)]
pub struct Node {
    pub name: String,
    pub parent: Option<u32>,
    pub children: Vec<u32>,
    pub kind: Kind,
    /// Subtree totals (own size for files).
    pub logical: u64,
    pub allocated: u64,
    /// Unix seconds.
    pub modified: i64,
    /// Newest modification anywhere in the subtree.
    pub newest: i64,
    pub files: u64,
    pub dirs: u64,
    pub unreadable: u64,
    pub is_unreadable: bool,
    /// Another volume or a pseudo filesystem, left out of the totals.
    pub is_skipped: bool,
    pub is_git: bool,
    /// A hard link to a file counted elsewhere; its size counts as 0.
    pub hard_dup: bool,
    /// Unix: st_mode. Windows: 0.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    /// Windows file attributes. Unix: 0.
    pub attrs: u32,
    pub git: Option<GitInfo>,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }

    pub fn size(&self, mode: SizeMode) -> u64 {
        match mode {
            SizeMode::OnDisk => self.allocated,
            SizeMode::Logical => self.logical,
        }
    }

    pub fn items(&self) -> u64 {
        self.files + self.dirs
    }
}

pub struct Tree {
    pub root_path: PathBuf,
    /// Index 0 is the scanned folder itself.
    pub nodes: Vec<Node>,
}

impl Tree {
    pub fn root(&self) -> &Node {
        &self.nodes[0]
    }

    pub fn path(&self, idx: u32) -> PathBuf {
        let mut parts: Vec<&str> = Vec::new();
        let mut cur = idx;
        while let Some(p) = self.nodes[cur as usize].parent {
            parts.push(&self.nodes[cur as usize].name);
            cur = p;
        }
        let mut path = self.root_path.clone();
        for part in parts.iter().rev() {
            path.push(part);
        }
        path
    }

    pub fn percent_of_parent(&self, idx: u32, mode: SizeMode) -> f64 {
        let node = &self.nodes[idx as usize];
        match node.parent {
            Some(p) if self.nodes[p as usize].size(mode) > 0 => {
                node.size(mode) as f64 / self.nodes[p as usize].size(mode) as f64
            }
            _ => 1.0,
        }
    }

    pub fn git_dir(&self, idx: u32) -> Option<u32> {
        self.nodes[idx as usize]
            .children
            .iter()
            .copied()
            .find(|&c| self.nodes[c as usize].name == ".git")
    }

    pub fn is_under(&self, idx: u32, ancestor: u32) -> bool {
        let mut cur = Some(idx);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.nodes[c as usize].parent;
        }
        false
    }

    /// Detach `idx` from its parent and subtract its totals from every ancestor.
    pub fn remove(&mut self, idx: u32) {
        let (logical, allocated, files, dirs, unreadable) = {
            let n = &self.nodes[idx as usize];
            let (f, d) = if n.is_dir() {
                (n.files, n.dirs + 1)
            } else {
                (1, 0)
            };
            (n.logical, n.allocated, f, d, n.unreadable)
        };
        let mut cur = self.nodes[idx as usize].parent;
        while let Some(p) = cur {
            let n = &mut self.nodes[p as usize];
            n.logical = n.logical.saturating_sub(logical);
            n.allocated = n.allocated.saturating_sub(allocated);
            n.files = n.files.saturating_sub(files);
            n.dirs = n.dirs.saturating_sub(dirs);
            n.unreadable = n.unreadable.saturating_sub(unreadable);
            cur = n.parent;
        }
        if let Some(p) = self.nodes[idx as usize].parent {
            self.nodes[p as usize].children.retain(|&c| c != idx);
        }
        self.nodes[idx as usize].parent = None;
    }

    /// The `limit` biggest files by allocated size.
    pub fn largest_files(&self, limit: usize) -> Vec<u32> {
        let mut files: Vec<u32> = (0..self.nodes.len() as u32)
            .filter(|&i| {
                !self.nodes[i as usize].is_dir() && self.nodes[i as usize].parent.is_some()
            })
            .collect();
        files.sort_by_key(|&i| std::cmp::Reverse(self.nodes[i as usize].allocated));
        files.truncate(limit);
        files
    }

    /// Every folder with a `.git` entry, biggest first; the root counts too.
    pub fn git_repos(&self) -> Vec<u32> {
        let mut repos: Vec<u32> = (0..self.nodes.len() as u32)
            .filter(|&i| self.nodes[i as usize].is_git)
            .filter(|&i| i == 0 || self.nodes[i as usize].parent.is_some())
            .collect();
        repos.sort_by_key(|&i| std::cmp::Reverse(self.nodes[i as usize].allocated));
        repos
    }
}

// MARK: scanning

#[derive(Default)]
pub struct Progress {
    pub items: AtomicUsize,
    pub bytes: AtomicU64,
    pub current: Mutex<String>,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Directory levels (from the root) whose subfolders are walked in parallel.
const PARALLEL_DEPTH: usize = 2;

struct Built {
    node: Node,
    children: Vec<Built>,
}

type Links = Mutex<HashSet<(u64, u64)>>;

/// Scan `root`; None when it cannot be read at all. Cancelling returns the partial tree.
pub fn scan(root: &Path, progress: &Progress) -> Option<Tree> {
    let meta = fs::symlink_metadata(root).ok()?;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let links: Links = Mutex::new(HashSet::new());
    let built = walk(root, name, &meta, 0, progress, &links);
    let mut nodes = Vec::with_capacity(progress.items.load(Ordering::Relaxed) + 1);
    flatten(built, None, &mut nodes);
    Some(Tree {
        root_path: root.to_path_buf(),
        nodes,
    })
}

fn flatten(built: Built, parent: Option<u32>, nodes: &mut Vec<Node>) -> u32 {
    let idx = nodes.len() as u32;
    let Built { mut node, children } = built;
    node.parent = parent;
    nodes.push(node);
    let ids: Vec<u32> = children
        .into_iter()
        .map(|c| flatten(c, Some(idx), nodes))
        .collect();
    nodes[idx as usize].children = ids;
    idx
}

fn unix_secs(t: std::io::Result<SystemTime>) -> i64 {
    t.ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

fn make_node(name: String, path: &Path, meta: &fs::Metadata, links: &Links) -> Node {
    let ft = meta.file_type();
    let kind = if ft.is_dir() {
        Kind::Dir
    } else if ft.is_symlink() {
        Kind::Symlink
    } else if ft.is_file() {
        Kind::File
    } else {
        Kind::Other
    };
    let raw = platform::raw(path, meta);
    let mut logical = if kind == Kind::Dir { 0 } else { meta.len() };
    let mut allocated = raw.allocated;
    let mut hard_dup = false;
    if kind != Kind::Dir
        && raw.nlink > 1
        && !links.lock().expect("links lock").insert((raw.dev, raw.ino))
    {
        hard_dup = true;
        logical = 0;
        allocated = 0;
    }
    let modified = unix_secs(meta.modified());
    Node {
        name,
        parent: None,
        children: Vec::new(),
        kind,
        logical,
        allocated,
        modified,
        newest: modified,
        files: 0,
        dirs: 0,
        unreadable: 0,
        is_unreadable: false,
        is_skipped: false,
        is_git: false,
        hard_dup,
        mode: raw.mode,
        uid: raw.uid,
        gid: raw.gid,
        attrs: raw.attrs,
        git: None,
    }
}

fn walk(
    path: &Path,
    name: String,
    meta: &fs::Metadata,
    depth: usize,
    progress: &Progress,
    links: &Links,
) -> Built {
    let mut node = make_node(name, path, meta, links);
    if !node.is_dir() {
        return Built {
            node,
            children: Vec::new(),
        };
    }
    if depth > 0 && skip_dir(path) {
        node.is_skipped = true;
        return Built {
            node,
            children: Vec::new(),
        };
    }
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) => {
            node.is_unreadable = true;
            node.unreadable = 1;
            return Built {
                node,
                children: Vec::new(),
            };
        }
    };

    let mut files: Vec<Built> = Vec::new();
    let mut dirs: Vec<(String, PathBuf, fs::Metadata)> = Vec::new();
    let mut unreadable = 0;
    let mut file_bytes = 0;
    for entry in rd {
        if progress.cancelled() {
            break;
        }
        let Ok(entry) = entry else {
            unreadable += 1;
            continue;
        };
        // DirEntry::metadata does not follow symlinks, so a link counts as itself.
        let Ok(meta) = entry.metadata() else {
            unreadable += 1;
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            node.is_git = true;
        }
        if meta.is_dir() {
            dirs.push((name, entry.path(), meta));
        } else {
            let child = make_node(name, &entry.path(), &meta, links);
            file_bytes += child.allocated;
            files.push(Built {
                node: child,
                children: Vec::new(),
            });
        }
    }
    progress
        .items
        .fetch_add(files.len() + dirs.len(), Ordering::Relaxed);
    progress.bytes.fetch_add(file_bytes, Ordering::Relaxed);
    if let Ok(mut cur) = progress.current.try_lock() {
        *cur = path.display().to_string();
    }

    let sub = |(name, p, m): (String, PathBuf, fs::Metadata)| {
        walk(&p, name, &m, depth + 1, progress, links)
    };
    let mut children: Vec<Built> = if depth < PARALLEL_DEPTH && dirs.len() > 1 {
        dirs.into_par_iter().map(sub).collect()
    } else {
        dirs.into_iter().map(sub).collect()
    };
    children.append(&mut files);

    for c in &children {
        node.logical += c.node.logical;
        node.allocated += c.node.allocated;
        unreadable += c.node.unreadable;
        if c.node.is_dir() {
            node.dirs += 1 + c.node.dirs;
            node.files += c.node.files;
        } else {
            node.files += 1;
        }
        node.newest = node.newest.max(c.node.newest);
    }
    node.unreadable = unreadable;
    Built { node, children }
}

/// Mount points of other volumes and pseudo filesystems: scanning `/` must not count them, and on
/// macOS the Data volume is already reachable through firmlinks such as /Users.
pub fn skip_dir(path: &Path) -> bool {
    const SKIP: &[&str] = if cfg!(target_os = "macos") {
        &["/System/Volumes", "/Volumes", "/dev"]
    } else if cfg!(windows) {
        &[]
    } else {
        &["/proc", "/sys", "/dev", "/run"]
    };
    SKIP.iter().any(|s| Path::new(s) == path)
}

// MARK: platform details

pub struct Raw {
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub attrs: u32,
    pub allocated: u64,
    pub nlink: u64,
    pub dev: u64,
    pub ino: u64,
}

#[cfg(unix)]
mod platform {
    use super::Raw;
    use std::collections::HashMap;
    use std::ffi::CStr;
    use std::fs::Metadata;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;
    use std::sync::Mutex;

    pub fn raw(_path: &Path, m: &Metadata) -> Raw {
        Raw {
            mode: m.mode(),
            uid: m.uid(),
            gid: m.gid(),
            attrs: 0,
            allocated: m.blocks() * 512,
            nlink: m.nlink(),
            dev: m.dev(),
            ino: m.ino(),
        }
    }

    static USERS: Mutex<Option<HashMap<u32, String>>> = Mutex::new(None);
    static GROUPS: Mutex<Option<HashMap<u32, String>>> = Mutex::new(None);

    fn user_name(uid: u32) -> String {
        let mut cache = USERS.lock().expect("users lock");
        let cache = cache.get_or_insert_with(HashMap::new);
        if let Some(n) = cache.get(&uid) {
            return n.clone();
        }
        // SAFETY: getpwuid returns a pointer to static storage or null; copied out under our lock.
        let name = unsafe {
            let pw = libc::getpwuid(uid);
            if pw.is_null() {
                uid.to_string()
            } else {
                CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned()
            }
        };
        cache.insert(uid, name.clone());
        name
    }

    fn group_name(gid: u32) -> String {
        let mut cache = GROUPS.lock().expect("groups lock");
        let cache = cache.get_or_insert_with(HashMap::new);
        if let Some(n) = cache.get(&gid) {
            return n.clone();
        }
        // SAFETY: as for getpwuid.
        let name = unsafe {
            let gr = libc::getgrgid(gid);
            if gr.is_null() {
                gid.to_string()
            } else {
                CStr::from_ptr((*gr).gr_name).to_string_lossy().into_owned()
            }
        };
        cache.insert(gid, name.clone());
        name
    }

    /// "user:group" for a node.
    pub fn owner(_path: &Path, uid: u32, gid: u32) -> String {
        format!("{}:{}", user_name(uid), group_name(gid))
    }

    pub fn can_write(path: &Path) -> bool {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        // SAFETY: c is a valid NUL-terminated path.
        unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
    }
}

#[cfg(windows)]
mod platform {
    use super::Raw;
    use std::fs::Metadata;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::path::Path;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        LookupAccountSidW, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, GetCompressedFileSizeW, INVALID_FILE_SIZE,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    pub fn raw(path: &Path, m: &Metadata) -> Raw {
        let attrs = m.file_attributes();
        let allocated = if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
            0
        } else {
            // Compressed or sparse files take less than their length; round up to 4 KiB clusters.
            let w = wide(path);
            let mut high: u32 = 0;
            // SAFETY: w is NUL-terminated and high is a valid out pointer.
            let low = unsafe { GetCompressedFileSizeW(w.as_ptr(), &mut high) };
            let bytes = if low == INVALID_FILE_SIZE && high == 0 {
                m.len()
            } else {
                ((high as u64) << 32) | low as u64
            };
            bytes.div_ceil(4096) * 4096
        };
        Raw {
            mode: 0,
            uid: 0,
            gid: 0,
            attrs,
            allocated,
            nlink: 1,
            dev: 0,
            ino: 0,
        }
    }

    /// "DOMAIN\\user" for a path, or "" when the owner cannot be read.
    pub fn owner(path: &Path, _uid: u32, _gid: u32) -> String {
        let w = wide(path);
        let mut sid: PSID = null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: all pointers are valid for the call; sd is freed with LocalFree as documented.
        unsafe {
            let rc = GetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut sid,
                null_mut(),
                null_mut(),
                null_mut(),
                &mut sd,
            );
            if rc != 0 {
                return String::new();
            }
            let mut name_len: u32 = 0;
            let mut dom_len: u32 = 0;
            let mut use_: SID_NAME_USE = 0;
            LookupAccountSidW(
                null(),
                sid,
                null_mut(),
                &mut name_len,
                null_mut(),
                &mut dom_len,
                &mut use_,
            );
            let mut name = vec![0u16; name_len as usize];
            let mut dom = vec![0u16; dom_len as usize];
            let ok = LookupAccountSidW(
                null(),
                sid,
                name.as_mut_ptr(),
                &mut name_len,
                dom.as_mut_ptr(),
                &mut dom_len,
                &mut use_,
            );
            LocalFree(sd as HLOCAL);
            if ok == 0 {
                return String::new();
            }
            let text = |v: &[u16]| {
                let end = v.iter().position(|&c| c == 0).unwrap_or(v.len());
                String::from_utf16_lossy(&v[..end])
            };
            let (d, n) = (text(&dom), text(&name));
            if d.is_empty() { n } else { format!("{d}\\{n}") }
        }
    }

    pub fn can_write(path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|m| !m.permissions().readonly())
    }
}

pub fn owner(path: &Path, uid: u32, gid: u32) -> String {
    platform::owner(path, uid, gid)
}

pub fn can_write(path: &Path) -> bool {
    platform::can_write(path)
}

/// `drwxr-xr-x` from a Unix mode; on Windows a `d r h s a` attribute string.
pub fn perm_string(kind: Kind, mode: u32, attrs: u32) -> String {
    if cfg!(windows) {
        return windows_attr_string(kind, attrs);
    }
    unix_mode_string(kind, mode)
}

pub fn unix_mode_string(kind: Kind, mode: u32) -> String {
    let mut s = String::with_capacity(10);
    s.push(match kind {
        Kind::Dir => 'd',
        Kind::Symlink => 'l',
        Kind::File => '-',
        Kind::Other => match mode & 0o170000 {
            0o140000 => 's',
            0o010000 => 'p',
            0o020000 => 'c',
            0o060000 => 'b',
            _ => '?',
        },
    });
    let triplet = |s: &mut String, shift: u32, special: u32, special_char: char| {
        let bits = (mode >> shift) & 0o7;
        s.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        s.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        let x = bits & 0o1 != 0;
        s.push(match (mode & special != 0, x) {
            (true, true) => special_char,
            (true, false) => special_char.to_ascii_uppercase(),
            (false, true) => 'x',
            (false, false) => '-',
        });
    };
    triplet(&mut s, 6, 0o4000, 's');
    triplet(&mut s, 3, 0o2000, 's');
    triplet(&mut s, 0, 0o1000, 't');
    s
}

pub fn octal_string(mode: u32) -> String {
    format!("{:04o}", mode & 0o7777)
}

/// Windows attributes: d(irectory) r(ead-only) h(idden) s(ystem) a(rchive), '-' when unset.
pub fn windows_attr_string(kind: Kind, attrs: u32) -> String {
    const READONLY: u32 = 0x1;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    const ARCHIVE: u32 = 0x20;
    const REPARSE: u32 = 0x400;
    let mut s = String::with_capacity(5);
    s.push(match kind {
        Kind::Dir => 'd',
        Kind::Symlink => 'l',
        _ if attrs & REPARSE != 0 => 'l',
        _ => '-',
    });
    for (flag, c) in [
        (READONLY, 'r'),
        (HIDDEN, 'h'),
        (SYSTEM, 's'),
        (ARCHIVE, 'a'),
    ] {
        s.push(if attrs & flag != 0 { c } else { '-' });
    }
    s
}

// MARK: git

/// A console program run without its own window: once the GUI has no console, Windows would open one per call.
pub fn quiet_cmd(program: &str) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Whether `git` runs from PATH; checked once.
pub fn git_available() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        quiet_cmd("git")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// Branch, last commit, dirty state and remote of the repository at `dir`.
pub fn git_info(dir: &Path) -> GitInfo {
    if !git_available() {
        return git_fallback(dir);
    }
    // --no-optional-locks: never take index.lock in repos something else may be working in.
    // core.fsmonitor off: a repo's own config could name any program to run.
    let git = |args: &[&str]| {
        let out = quiet_cmd("git")
            .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let mut info = GitInfo::default();
    match git(&["rev-parse", "--abbrev-ref", "HEAD"]) {
        Some(b) if b == "HEAD" => {
            info.detached = true;
            info.branch =
                git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "detached".into());
        }
        Some(b) if !b.is_empty() => info.branch = b,
        // No commits yet, or not a repository git accepts.
        _ => info.branch = git_fallback(dir).branch,
    }
    if let Some(log) = git(&["log", "-1", "--format=%ct%x09%s"]) {
        let (ts, subject) = log.split_once('\t').unwrap_or((&log, ""));
        info.last_commit = ts.parse().ok();
        info.subject = subject.to_string();
    }
    info.changed = git(&["status", "--porcelain"]).map(|s| s.lines().count());
    info.remote = git(&["remote", "get-url", "origin"]).unwrap_or_default();
    info
}

/// Branch from .git/HEAD and its modification time, for machines without git.
fn git_fallback(dir: &Path) -> GitInfo {
    let mut info = GitInfo {
        branch: "?".into(),
        fallback: true,
        ..GitInfo::default()
    };
    let mut git_dir = dir.join(".git");
    // `.git` may be a file pointing at the real git dir (worktrees, submodules).
    if let Ok(text) = fs::read_to_string(&git_dir)
        && let Some(target) = text.strip_prefix("gitdir:")
    {
        let target = target.trim();
        git_dir = if Path::new(target).is_absolute() {
            PathBuf::from(target)
        } else {
            dir.join(target)
        };
    }
    let head = git_dir.join("HEAD");
    if let Ok(text) = fs::read_to_string(&head) {
        let line = text.trim();
        match line.strip_prefix("ref: refs/heads/") {
            Some(b) => info.branch = b.to_string(),
            None => {
                info.detached = true;
                info.branch = line.chars().take(8).collect();
            }
        }
    }
    if let Ok(m) = fs::metadata(&head) {
        info.last_commit = Some(unix_secs(m.modified())).filter(|&t| t > 0);
    }
    info
}

// MARK: volumes

#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    pub mount: PathBuf,
    /// Label or device.
    pub name: String,
    pub fs: String,
    pub total: u64,
    pub used: u64,
    /// Free for the current user.
    pub free: u64,
}

pub fn volumes() -> Vec<Volume> {
    let mut v = volumes_impl::list();
    v.sort_by(|a, b| a.mount.cmp(&b.mount));
    v
}

/// The volume that holds `path`.
pub fn volume_of(path: &Path) -> Option<Volume> {
    volumes_impl::of(path)
}

#[cfg(target_os = "macos")]
mod volumes_impl {
    use super::Volume;
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    fn text(bytes: &[libc::c_char]) -> String {
        // SAFETY: the statfs name fields are NUL-terminated within their fixed size.
        unsafe { CStr::from_ptr(bytes.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    fn from_statfs(s: &libc::statfs) -> Volume {
        let bs = s.f_bsize as u64;
        let mount = PathBuf::from(text(&s.f_mntonname));
        let name = match mount.to_str() {
            Some("/") => "System".to_string(),
            Some("/System/Volumes/Data") => "Data (home folders)".to_string(),
            _ => mount
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| text(&s.f_mntfromname)),
        };
        Volume {
            mount,
            name,
            fs: text(&s.f_fstypename),
            total: s.f_blocks * bs,
            used: s.f_blocks.saturating_sub(s.f_bfree) * bs,
            free: s.f_bavail * bs,
        }
    }

    fn interesting(v: &Volume) -> bool {
        if v.total == 0 || matches!(v.fs.as_str(), "devfs" | "autofs" | "nullfs" | "fdesc") {
            return false;
        }
        let m = v.mount.to_string_lossy();
        m == "/" || m == "/System/Volumes/Data" || !m.starts_with("/System/Volumes/")
    }

    pub fn list() -> Vec<Volume> {
        let mut mounts: *mut libc::statfs = std::ptr::null_mut();
        // SAFETY: getmntinfo fills a buffer it owns; we only read `n` entries from it.
        let n = unsafe { libc::getmntinfo(&mut mounts, libc::MNT_NOWAIT) };
        if n <= 0 || mounts.is_null() {
            return Vec::new();
        }
        let all = unsafe { std::slice::from_raw_parts(mounts, n as usize) };
        all.iter().map(from_statfs).filter(interesting).collect()
    }

    pub fn of(path: &Path) -> Option<Volume> {
        let c = CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: c is NUL-terminated and s is a valid out buffer.
        let mut s: libc::statfs = unsafe { std::mem::zeroed() };
        (unsafe { libc::statfs(c.as_ptr(), &mut s) } == 0).then(|| from_statfs(&s))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod volumes_impl {
    use super::Volume;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    fn statvfs(mount: &Path) -> Option<(u64, u64, u64)> {
        let c = CString::new(mount.as_os_str().as_bytes()).ok()?;
        // SAFETY: c is NUL-terminated and s is a valid out buffer.
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
            return None;
        }
        let bs = s.f_frsize as u64;
        Some((
            s.f_blocks as u64 * bs,
            (s.f_blocks as u64).saturating_sub(s.f_bfree as u64) * bs,
            s.f_bavail as u64 * bs,
        ))
    }

    pub fn list() -> Vec<Volume> {
        const REAL: &[&str] = &[
            "ext2", "ext3", "ext4", "xfs", "btrfs", "zfs", "f2fs", "vfat", "exfat", "ntfs",
            "ntfs3", "fuseblk", "nfs", "nfs4", "cifs", "smb3",
        ];
        let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") else {
            return Vec::new();
        };
        mounts
            .lines()
            .filter_map(|l| {
                let mut f = l.split_whitespace();
                let (dev, mount, fs) = (f.next()?, f.next()?, f.next()?);
                if !REAL.contains(&fs) {
                    return None;
                }
                let mount = PathBuf::from(mount.replace("\\040", " "));
                let (total, used, free) = statvfs(&mount)?;
                (total > 0).then(|| Volume {
                    name: mount
                        .file_name()
                        .map_or_else(|| dev.to_string(), |n| n.to_string_lossy().into_owned()),
                    mount,
                    fs: fs.to_string(),
                    total,
                    used,
                    free,
                })
            })
            .collect()
    }

    pub fn of(path: &Path) -> Option<Volume> {
        list()
            .into_iter()
            .filter(|v| path.starts_with(&v.mount))
            .max_by_key(|v| v.mount.as_os_str().len())
    }
}

#[cfg(windows)]
mod volumes_impl {
    use super::Volume;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Path, PathBuf};
    use std::ptr::null_mut;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };

    // GetDriveTypeW results worth listing (winbase.h).
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn utf16(v: &[u16]) -> String {
        let end = v.iter().position(|&c| c == 0).unwrap_or(v.len());
        String::from_utf16_lossy(&v[..end])
    }

    fn volume(root: PathBuf) -> Option<Volume> {
        let w = wide(&root);
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        // SAFETY: w is NUL-terminated; the out pointers are valid u64s.
        if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) } == 0 {
            return None;
        }
        let mut label = [0u16; 261];
        let mut fs = [0u16; 261];
        // SAFETY: buffers are sized as passed; unneeded out params may be null.
        let ok = unsafe {
            GetVolumeInformationW(
                w.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                null_mut(),
                null_mut(),
                null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        let (label, fs) = if ok == 0 {
            (String::new(), String::new())
        } else {
            (utf16(&label), utf16(&fs))
        };
        let drive = root.to_string_lossy().trim_end_matches('\\').to_string();
        Some(Volume {
            name: if label.is_empty() {
                drive
            } else {
                format!("{label} ({drive})")
            },
            mount: root,
            fs,
            total,
            used: total.saturating_sub(free),
            free: avail,
        })
    }

    pub fn list() -> Vec<Volume> {
        // SAFETY: no arguments; returns a bitmask of drive letters.
        let mask = unsafe { GetLogicalDrives() };
        (0..26u32)
            .filter(|i| mask & (1 << i) != 0)
            .filter_map(|i| {
                let root = PathBuf::from(format!("{}:\\", (b'A' + i as u8) as char));
                let w = wide(&root);
                // SAFETY: w is NUL-terminated.
                let kind = unsafe { GetDriveTypeW(w.as_ptr()) };
                matches!(kind, DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_REMOTE)
                    .then(|| volume(root))
                    .flatten()
            })
            .collect()
    }

    pub fn of(path: &Path) -> Option<Volume> {
        let mut root = PathBuf::new();
        for c in path.components().take(2) {
            match c {
                Component::Prefix(_) | Component::RootDir => root.push(c),
                _ => break,
            }
        }
        if root.as_os_str().is_empty() {
            return None;
        }
        volume(root)
    }
}

// MARK: file operations (Move, Trash, Delete)

pub mod ops {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub fn trash(path: &Path) -> Result<(), String> {
        trash::delete(path).map_err(|e| e.to_string())
    }

    pub fn delete_permanently(path: &Path) -> Result<(), String> {
        let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
        .map_err(|e| e.to_string())
    }

    /// Move `src` into `dest_dir`, copying and deleting when the destination is on another volume.
    /// `files` counts copied files for progress. Returns the new path.
    pub fn move_into(src: &Path, dest_dir: &Path, files: &AtomicUsize) -> Result<PathBuf, String> {
        let name = src.file_name().ok_or("cannot move a root folder")?;
        let dest = dest_dir.join(name);
        if dest.exists() {
            return Err(format!("{} already exists", dest.display()));
        }
        if dest.starts_with(src) {
            return Err("cannot move a folder into itself".into());
        }
        match fs::rename(src, &dest) {
            Ok(()) => Ok(dest),
            Err(e) if cross_device(&e) => {
                copy_tree(src, &dest, files)?;
                delete_permanently(src)?;
                Ok(dest)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    fn cross_device(e: &std::io::Error) -> bool {
        // EXDEV on Unix, ERROR_NOT_SAME_DEVICE on Windows.
        e.raw_os_error() == Some(if cfg!(windows) { 17 } else { 18 })
    }

    fn copy_tree(src: &Path, dest: &Path, files: &AtomicUsize) -> Result<(), String> {
        let meta = fs::symlink_metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
        let ft = meta.file_type();
        if ft.is_symlink() {
            let target = fs::read_link(src).map_err(|e| format!("{}: {e}", src.display()))?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, dest)
                .map_err(|e| format!("{}: {e}", dest.display()))?;
            #[cfg(windows)]
            {
                let _ = target;
                return Err(format!(
                    "{}: symbolic links are not copied on Windows",
                    src.display()
                ));
            }
            #[cfg(not(windows))]
            return Ok(());
        }
        if ft.is_dir() {
            fs::create_dir(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            for entry in fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
                let entry = entry.map_err(|e| e.to_string())?;
                copy_tree(&entry.path(), &dest.join(entry.file_name()), files)?;
            }
            return Ok(());
        }
        fs::copy(src, dest).map_err(|e| format!("{}: {e}", src.display()))?;
        if let Ok(m) = meta.modified() {
            let _ = fs::File::options()
                .write(true)
                .open(dest)
                .and_then(|f| f.set_modified(m));
        }
        files.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn moves_within_a_volume_and_refuses_existing_targets() {
            let dir =
                std::env::temp_dir().join(format!("disk-explorer-ops-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("a/inner")).unwrap();
            fs::create_dir_all(dir.join("b")).unwrap();
            fs::write(dir.join("a/inner/f.txt"), b"hi").unwrap();
            let files = AtomicUsize::new(0);
            let moved = move_into(&dir.join("a"), &dir.join("b"), &files).unwrap();
            assert_eq!(moved, dir.join("b/a"));
            assert!(dir.join("b/a/inner/f.txt").exists());
            assert!(!dir.join("a").exists());
            fs::create_dir_all(dir.join("a")).unwrap();
            assert!(move_into(&dir.join("a"), &dir.join("b"), &files).is_err());
            assert!(move_into(&dir.join("b"), &dir.join("b/a"), &files).is_err());
            // A copied tree keeps its files and counts them.
            copy_tree(&dir.join("b"), &dir.join("c"), &files).unwrap();
            assert_eq!(files.load(Ordering::Relaxed), 1);
            assert!(dir.join("c/a/inner/f.txt").exists());
            delete_permanently(&dir).unwrap();
            assert!(!dir.exists());
        }
    }
}

// MARK: Trash / Recycle Bin

pub mod bin {
    use std::path::PathBuf;

    /// Restore works through the OS on Windows and Linux; macOS keeps "Put Back" inside Finder.
    pub const CAN_RESTORE: bool = !cfg!(target_os = "macos");

    #[derive(Clone, Debug)]
    pub struct Item {
        pub name: String,
        pub original: Option<PathBuf>,
        /// Unix seconds.
        pub deleted: i64,
        /// Where the item is stored now, for sizing and reveal.
        pub path: Option<PathBuf>,
        #[cfg(not(target_os = "macos"))]
        raw: trash::TrashItem,
    }

    #[cfg(not(target_os = "macos"))]
    pub fn list() -> Result<Vec<Item>, String> {
        let items = trash::os_limited::list().map_err(|e| e.to_string())?;
        Ok(items
            .into_iter()
            .map(|t| Item {
                name: t.name.to_string_lossy().into_owned(),
                original: Some(t.original_path()),
                deleted: t.time_deleted,
                path: storage_path(&t),
                raw: t,
            })
            .collect())
    }

    #[cfg(not(target_os = "macos"))]
    fn storage_path(t: &trash::TrashItem) -> Option<PathBuf> {
        let p = if cfg!(windows) {
            // The parsing name of a recycled file is its $R path.
            PathBuf::from(&t.id)
        } else {
            // id is .../info/<x>.trashinfo and the data lives at .../files/<x>.
            let info = std::path::Path::new(&t.id);
            let stem = info
                .file_name()?
                .to_string_lossy()
                .strip_suffix(".trashinfo")?
                .to_string();
            info.parent()?.parent()?.join("files").join(stem)
        };
        p.exists().then_some(p)
    }

    #[cfg(not(target_os = "macos"))]
    pub fn restore(items: &[Item]) -> Result<(), String> {
        trash::os_limited::restore_all(items.iter().map(|i| i.raw.clone()))
            .map_err(|e| e.to_string())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn purge(items: &[Item]) -> Result<(), String> {
        trash::os_limited::purge_all(items.iter().map(|i| i.raw.clone())).map_err(|e| e.to_string())
    }

    #[cfg(target_os = "macos")]
    pub fn list() -> Result<Vec<Item>, String> {
        use std::fs;
        use std::os::unix::fs::MetadataExt;
        let home = std::env::home_dir().ok_or("no home folder")?;
        let main = home.join(".Trash");
        let mut dirs = vec![main.clone()];
        // External volumes keep a per-user trash in /Volumes/<name>/.Trashes/<uid>.
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        for e in fs::read_dir("/Volumes").into_iter().flatten().flatten() {
            let p = e.path().join(".Trashes").join(uid.to_string());
            if p.is_dir() {
                dirs.push(p);
            }
        }
        let mut out = Vec::new();
        let mut error = None;
        for dir in dirs {
            match fs::read_dir(&dir) {
                Ok(rd) => {
                    for e in rd.flatten() {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if name == ".DS_Store" {
                            continue;
                        }
                        let Ok(meta) = e.metadata() else { continue };
                        out.push(Item {
                            name,
                            original: None,
                            // The inode change time is when the item was moved into the Trash.
                            deleted: meta.ctime(),
                            path: Some(e.path()),
                        });
                    }
                }
                Err(e) if dir == main => {
                    error = Some(format!(
                        "{}: {e}. Grant Full Disk Access in System Settings › Privacy & Security to see the Trash.",
                        dir.display()
                    ));
                }
                Err(_) => {}
            }
        }
        match error {
            Some(e) if out.is_empty() => Err(e),
            _ => Ok(out),
        }
    }

    #[cfg(target_os = "macos")]
    pub fn restore(_items: &[Item]) -> Result<(), String> {
        Err("On macOS, use Put Back in the Finder: Reveal opens the item in the Trash.".into())
    }

    #[cfg(target_os = "macos")]
    pub fn purge(items: &[Item]) -> Result<(), String> {
        for i in items {
            if let Some(p) = &i.path {
                super::ops::delete_permanently(p)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_mode_strings() {
        assert_eq!(unix_mode_string(Kind::Dir, 0o40755), "drwxr-xr-x");
        assert_eq!(unix_mode_string(Kind::File, 0o100644), "-rw-r--r--");
        assert_eq!(unix_mode_string(Kind::File, 0o104755), "-rwsr-xr-x");
        assert_eq!(unix_mode_string(Kind::File, 0o104644), "-rwSr--r--");
        assert_eq!(unix_mode_string(Kind::Dir, 0o41777), "drwxrwxrwt");
        assert_eq!(unix_mode_string(Kind::Symlink, 0o120777), "lrwxrwxrwx");
        assert_eq!(octal_string(0o100644), "0644");
    }

    #[test]
    fn windows_attr_strings() {
        assert_eq!(windows_attr_string(Kind::Dir, 0x10), "d----");
        assert_eq!(windows_attr_string(Kind::File, 0x1 | 0x2 | 0x20), "-rh-a");
        assert_eq!(windows_attr_string(Kind::File, 0x400), "l----");
    }

    fn file(name: &str, parent: u32, size: u64) -> Node {
        Node {
            name: name.into(),
            parent: Some(parent),
            children: vec![],
            kind: Kind::File,
            logical: size,
            allocated: size,
            modified: 0,
            newest: 0,
            files: 0,
            dirs: 0,
            unreadable: 0,
            is_unreadable: false,
            is_skipped: false,
            is_git: false,
            hard_dup: false,
            mode: 0,
            uid: 0,
            gid: 0,
            attrs: 0,
            git: None,
        }
    }

    #[test]
    fn remove_subtracts_from_ancestors_and_detaches() {
        let mut root = file("root", 0, 30);
        root.parent = None;
        root.kind = Kind::Dir;
        root.children = vec![1, 2];
        root.files = 2;
        let mut t = Tree {
            root_path: PathBuf::from("/r"),
            nodes: vec![root, file("a", 0, 10), file("b", 0, 20)],
        };
        assert_eq!(t.path(2), PathBuf::from("/r/b"));
        t.remove(2);
        assert_eq!(t.root().allocated, 10);
        assert_eq!(t.root().files, 1);
        assert_eq!(t.root().children, vec![1]);
        assert_eq!(t.nodes[2].parent, None);
        assert!(t.is_under(1, 0));
        assert!(!t.is_under(2, 0));
    }

    #[test]
    fn scans_a_temp_tree() {
        let dir = std::env::temp_dir().join(format!("disk-explorer-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub/.git")).unwrap();
        fs::write(dir.join("sub/file.txt"), vec![b'x'; 5000]).unwrap();
        fs::write(dir.join("top.bin"), vec![b'y'; 100]).unwrap();
        let progress = Progress::default();
        let tree = scan(&dir, &progress).unwrap();
        let root = tree.root();
        assert_eq!(root.files, 2);
        assert_eq!(root.dirs, 2);
        assert_eq!(root.logical, 5100);
        assert!(root.allocated >= 5100);
        let sub = tree.git_repos();
        assert_eq!(sub.len(), 1);
        assert_eq!(tree.nodes[sub[0] as usize].name, "sub");
        assert_eq!(tree.largest_files(1).len(), 1);
        assert_eq!(
            tree.nodes[tree.largest_files(1)[0] as usize].name,
            "file.txt"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
