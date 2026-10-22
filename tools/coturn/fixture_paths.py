"""Paths for root-owned, loopback-only coturn reproduction artifacts."""
import os
import pathlib
import stat

def fixture_root():
    raw = os.environ.get("COTURN_FIXTURE_ROOT")
    if not raw:
        raise RuntimeError("Set COTURN_FIXTURE_ROOT to a new private directory outside the checkout")
    root = pathlib.Path(raw)
    if not root.is_absolute() or ".." in root.parts:
        raise RuntimeError("COTURN_FIXTURE_ROOT must be an absolute path without '..'")
    meta = root.lstat()
    if os.geteuid() != 0 or not stat.S_ISDIR(meta.st_mode) or root.is_symlink():
        raise RuntimeError("The fixture requires root and a real directory")
    if meta.st_uid != 0 or stat.S_IMODE(meta.st_mode) != 0o700 or root.resolve() != root:
        raise RuntimeError("The fixture directory must be root-owned mode 0700 without symlink parents")
    return root
