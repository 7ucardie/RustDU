# How scanning works

RustDU walks the selected folder and adds up file sizes for each directory. It reports allocated size
(space on disk) and logical size, counts hard links once, and flags folders it cannot read.

For whole-volume scans, mount points of other volumes and pseudo filesystems are listed but not traversed.
On macOS this also avoids counting the Data volume twice through firmlinks. Full Disk Access may be needed
to scan protected locations.

Git information comes from the `git` on your PATH with optional locks disabled. If git is unavailable,
RustDU reads the branch from `.git/HEAD` and leaves the dirty state unknown.
