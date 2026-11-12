//! The processes the oracle starts, and stopping them with everything they
//! started in turn.
//!
//! On Unix each run of the program leads a process group of its own, so that
//! a timeout kills the group, including what a wrapper script started.

use std::process::Child;
use std::process::Command;

/// Makes the program run by `command` lead a process group of its own.
pub(super) fn own_group(command: &mut Command) {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(command, 0);
    #[cfg(not(unix))]
    let _ = command;
}

/// A running program's process group.
pub(super) struct Group {
    #[cfg(unix)]
    id: i32,
}

impl Group {
    /// The group `child` leads.
    pub(super) fn of(child: &Child) -> Group {
        #[cfg(unix)]
        {
            Group {
                id: i32::try_from(child.id()).unwrap_or(0),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = child;
            Group {}
        }
    }

    /// Kills every process of the group, and `child`, which leads it. Call
    /// before reaping `child`, or while something else of its group is
    /// known to run, so that the group's id cannot have been reused.
    pub(super) fn kill(&self, child: &mut Child) {
        #[cfg(unix)]
        if self.id > 0 {
            // SAFETY: `kill` takes no pointers; a group that has gone is
            // ESRCH.
            unsafe {
                libc::kill(-self.id, libc::SIGKILL);
            }
        }
        let _ = child.kill();
    }
}
