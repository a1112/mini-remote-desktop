//! Process launching for background infrastructure tasks.

use std::{ffi::OsStr, process::Command};

/// Create a background command without allocating a Windows console.
///
/// The caller still controls arguments, standard streams, spawning and child cleanup.
/// Interactive terminal commands should continue to use `Command::new` directly.
pub fn background_command(program: impl AsRef<OsStr>) -> Command {
    let command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = command;
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    {
        command
    }
}
