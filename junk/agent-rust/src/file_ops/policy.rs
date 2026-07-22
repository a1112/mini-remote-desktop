use crate::control_plane::mount_protocol;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilePolicy {
    pub read_only: bool,
    pub allow_delete: bool,
    pub allow_move: bool,
    pub allow_overwrite: bool,
}

impl FilePolicy {
    pub fn from_mount_flags(flags: u32) -> Self {
        Self {
            read_only: (flags & mount_protocol::FLAG_READ_ONLY) != 0,
            allow_delete: (flags & mount_protocol::FLAG_ALLOW_DELETE) != 0,
            allow_move: (flags & mount_protocol::FLAG_ALLOW_MOVE) != 0,
            allow_overwrite: (flags & mount_protocol::FLAG_ALLOW_OVERWRITE) != 0,
        }
    }
}

impl Default for FilePolicy {
    fn default() -> Self {
        Self {
            read_only: false,
            allow_delete: true,
            allow_move: true,
            allow_overwrite: true,
        }
    }
}
