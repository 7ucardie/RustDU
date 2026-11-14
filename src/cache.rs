//! Keeps the last scan on disk so the next launch opens with it instead of an empty window.
//! The file lives in the user's cache folder, never in the scanned tree.

use crate::scan::{Kind, Node, Tree};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Bump the last byte when the layout below changes: older files are then ignored.
const MAGIC: &[u8; 8] = b"RUSTDU\0\x01";

pub struct Saved {
    pub tree: Tree,
    /// Unix seconds when the scan finished.
    pub scanned_at: i64,
    /// The scan was stopped early.
    pub partial: bool,
}

fn put(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_signed(out: &mut Vec<u8>, v: i64) {
    put(out, ((v << 1) ^ (v >> 63)) as u64);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

/// Git details are left out: they are probed again on load, so they are never stale.
pub fn encode(tree: &Tree, scanned_at: i64, partial: bool) -> Vec<u8> {
    // Only what is still reachable from the root: moved and trashed items stay in `nodes` detached.
    let mut order: Vec<u32> = Vec::with_capacity(tree.nodes.len());
    let mut new_index = vec![u32::MAX; tree.nodes.len()];
    let mut stack = vec![0u32];
    while let Some(i) = stack.pop() {
        new_index[i as usize] = order.len() as u32;
        order.push(i);
        // Reversed so children keep their order: decode rebuilds `children` from the parents.
        stack.extend(tree.nodes[i as usize].children.iter().rev());
    }

    let mut out = Vec::with_capacity(order.len() * 48 + 64);
    out.extend_from_slice(MAGIC);
    put_signed(&mut out, scanned_at);
    out.push(partial as u8);
    put_str(&mut out, &tree.root_path.to_string_lossy());
    put(&mut out, order.len() as u64);
    for &i in &order {
        let n = &tree.nodes[i as usize];
        put_str(&mut out, &n.name);
        if let Some(p) = n.parent.filter(|_| i != 0) {
            put(&mut out, new_index[p as usize] as u64);
        }
        let kind = match n.kind {
            Kind::Dir => 0u8,
            Kind::File => 1,
            Kind::Symlink => 2,
            Kind::Other => 3,
        };
        out.push(
            kind | (n.is_unreadable as u8) << 2
                | (n.is_skipped as u8) << 3
                | (n.is_git as u8) << 4
                | (n.hard_dup as u8) << 5,
        );
        put(&mut out, n.logical);
        put(&mut out, n.allocated);
        put_signed(&mut out, n.modified);
        put_signed(&mut out, n.newest.wrapping_sub(n.modified));
        put(&mut out, n.files);
        put(&mut out, n.dirs);
        put(&mut out, n.unreadable);
        put(&mut out, n.mode as u64);
        put(&mut out, n.uid as u64);
        put(&mut out, n.gid as u64);
        put(&mut out, n.attrs as u64);
    }
    out
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    fn num(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            v |= u64::from(b & 0x7F) << shift;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }

    fn signed(&mut self) -> Option<i64> {
        let v = self.num()?;
        Some((v >> 1) as i64 ^ -((v & 1) as i64))
    }

    fn small(&mut self) -> Option<u32> {
        u32::try_from(self.num()?).ok()
    }

    fn string(&mut self) -> Option<String> {
        let len = usize::try_from(self.num()?).ok()?;
        let end = self.pos.checked_add(len)?;
        let s = std::str::from_utf8(self.bytes.get(self.pos..end)?).ok()?;
        self.pos = end;
        Some(s.to_string())
    }
}

/// None for anything that is not a complete file of this version: a damaged cache is just no cache.
pub fn decode(bytes: &[u8]) -> Option<Saved> {
    if !bytes.starts_with(MAGIC) {
        return None;
    }
    let mut r = Reader {
        bytes,
        pos: MAGIC.len(),
    };
    let scanned_at = r.signed()?;
    let partial = r.byte()? != 0;
    let root_path = PathBuf::from(r.string()?);
    let count = usize::try_from(r.num()?).ok()?;
    // Every node takes more than a byte, so a count beyond the file size is corrupt, not a huge allocation.
    if count == 0 || count > bytes.len() {
        return None;
    }
    let mut nodes: Vec<Node> = Vec::with_capacity(count);
    for i in 0..count {
        let name = r.string()?;
        let parent = if i == 0 {
            None
        } else {
            let p = r.small()?;
            if p as usize >= i {
                return None;
            }
            Some(p)
        };
        let flags = r.byte()?;
        let kind = match flags & 3 {
            0 => Kind::Dir,
            1 => Kind::File,
            2 => Kind::Symlink,
            _ => Kind::Other,
        };
        let logical = r.num()?;
        let allocated = r.num()?;
        let modified = r.signed()?;
        let newest = modified.wrapping_add(r.signed()?);
        if let Some(p) = parent {
            nodes[p as usize].children.push(i as u32);
        }
        nodes.push(Node {
            name,
            parent,
            children: Vec::new(),
            kind,
            logical,
            allocated,
            modified,
            newest,
            files: r.num()?,
            dirs: r.num()?,
            unreadable: r.num()?,
            is_unreadable: flags & 4 != 0,
            is_skipped: flags & 8 != 0,
            is_git: flags & 16 != 0,
            hard_dup: flags & 32 != 0,
            mode: r.small()?,
            uid: r.small()?,
            gid: r.small()?,
            attrs: r.small()?,
            git: None,
        });
    }
    Some(Saved {
        tree: Tree { root_path, nodes },
        scanned_at,
        partial,
    })
}

fn dir() -> Option<PathBuf> {
    let env = |key: &str| {
        std::env::var_os(key)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(target_os = "macos") {
        Some(env("HOME")?.join("Library/Caches/RustDU"))
    } else if cfg!(windows) {
        Some(env("LOCALAPPDATA")?.join("RustDU"))
    } else {
        let base = env("XDG_CACHE_HOME").or_else(|| Some(env("HOME")?.join(".cache")))?;
        Some(base.join("rustdu"))
    }
}

fn file() -> Option<PathBuf> {
    Some(dir()?.join("last-scan.bin"))
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    // The file lists every name under the scanned folder: keep it to this user.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)?.write_all(bytes)
}

static NEXT: AtomicU64 = AtomicU64::new(1);
/// Number of the newest save on disk; also keeps two saves from writing at once.
static WRITTEN: Mutex<u64> = Mutex::new(0);

/// Write `bytes` (from `encode`) in the background. Best effort: without a cache the app just starts empty.
pub fn save(bytes: Vec<u8>) {
    let Some(path) = file() else { return };
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    std::thread::spawn(move || {
        let mut written = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
        if seq < *written {
            return;
        }
        // Written beside the target and renamed, so quitting mid-write leaves the previous cache intact.
        let tmp = path.with_extension("tmp");
        let done = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| write_private(&tmp, &bytes))
            .and_then(|()| fs::rename(&tmp, &path));
        if done.is_ok() {
            *written = seq;
        }
    });
}

pub fn load() -> Option<Saved> {
    decode(&fs::read(file()?).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, parent: Option<u32>, kind: Kind, size: u64) -> Node {
        Node {
            name: name.into(),
            parent,
            children: Vec::new(),
            kind,
            logical: size,
            allocated: size + 96,
            modified: 1_700_000_000,
            newest: 1_700_000_500,
            files: 0,
            dirs: 0,
            unreadable: 0,
            is_unreadable: false,
            is_skipped: false,
            is_git: false,
            hard_dup: false,
            mode: 0o100644,
            uid: 501,
            gid: 20,
            attrs: 0,
            git: None,
        }
    }

    fn sample() -> Tree {
        let mut nodes = vec![
            node("root", None, Kind::Dir, 600),
            node("a", Some(0), Kind::Dir, 300),
            node("één.txt", Some(1), Kind::File, 300),
            node("gone", Some(0), Kind::Dir, 200),
            node("inside", Some(3), Kind::File, 200),
            node("link", Some(0), Kind::Symlink, 100),
        ];
        nodes[0].children = vec![1, 3, 5];
        nodes[1].children = vec![2];
        nodes[3].children = vec![4];
        nodes[0].is_git = true;
        nodes[0].files = 2;
        nodes[0].dirs = 2;
        nodes[2].hard_dup = true;
        nodes[5].modified = -5;
        nodes[5].newest = -5;
        Tree {
            root_path: PathBuf::from("/tmp/root"),
            nodes,
        }
    }

    #[test]
    fn round_trips_and_drops_removed_items() {
        let mut tree = sample();
        tree.remove(3);
        let saved = decode(&encode(&tree, 1_763_000_000, true)).expect("decodes");
        assert_eq!(saved.scanned_at, 1_763_000_000);
        assert!(saved.partial);
        let t = saved.tree;
        assert_eq!(t.root_path, PathBuf::from("/tmp/root"));
        let names: Vec<&str> = t.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["root", "a", "één.txt", "link"]);
        assert_eq!(t.nodes[0].children, [1, 3]);
        assert_eq!(t.nodes[1].children, [2]);
        assert_eq!(t.nodes[2].parent, Some(1));
        assert!(t.nodes[0].is_git && t.nodes[2].hard_dup);
        assert_eq!(t.nodes[0].logical, 400);
        assert_eq!(t.nodes[2].allocated, 396);
        assert_eq!(t.nodes[2].newest, 1_700_000_500);
        assert_eq!((t.nodes[3].kind, t.nodes[3].modified), (Kind::Symlink, -5));
        assert_eq!((t.nodes[2].mode, t.nodes[2].uid), (0o100644, 501));
        assert_eq!(t.path(2), PathBuf::from("/tmp/root/a/één.txt"));
    }

    #[test]
    fn damaged_files_are_rejected() {
        let bytes = encode(&sample(), 1, false);
        assert!(decode(&bytes).is_some());
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).is_none(), "truncated at {cut}");
        }
        assert!(decode(b"not a cache").is_none());
        let mut other_version = bytes.clone();
        other_version[7] = 0xFF;
        assert!(decode(&other_version).is_none());
    }
}
