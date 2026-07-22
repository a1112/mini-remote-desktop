pub const MOUNT_OPEN: u8 = 0x01;
pub const MOUNT_LIST: u8 = 0x02;
pub const MOUNT_CLOSE: u8 = 0x03;
pub const MOUNT_HEARTBEAT: u8 = 0x04;
pub const MOUNT_CAPS_QUERY: u8 = 0x05;

pub const FLAG_READ_ONLY: u32 = 1 << 0;
pub const FLAG_AUTO_CREATE_ROOT: u32 = 1 << 1;
pub const FLAG_ALLOW_DELETE: u32 = 1 << 2;
pub const FLAG_ALLOW_MOVE: u32 = 1 << 3;
pub const FLAG_ALLOW_OVERWRITE: u32 = 1 << 4;
pub const FLAG_STRICT_ETAG: u32 = 1 << 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MountErrorCode {
    InvalidOp = 1001,
    InvalidField = 1002,
    MountNotFound = 2001,
    MountAlreadyExists = 2002,
    MountStateConflict = 2003,
    HeartbeatTimeout = 2004,
    DavUnavailable = 3002,
    PathForbidden = 4001,
}
