//! The processes the oracle starts, and stopping them with everything they
//! started in turn.
//!
//! On Unix each run of the program leads a process group of its own, so that
//! a timeout kills the group, including what a wrapper script started. A
//! group is not in the terminal's foreground group, so Ctrl-C does not reach
//! it: [`stop_running_on_signals`] makes `SIGINT`, `SIGTERM` and `SIGHUP`
//! kill the running groups before chairlift itself stops.

use std::process::Child;
use std::process::Command;

/// Makes the program run by `command` lead a process group of its own.
pub(super) fn own_group(command: &mut Command) {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(command, 0);
    #[cfg(not(unix))]
    let _ = command;
}

/// A running program's process group, listed for [`kill_running`] while
/// this lives.
pub(super) struct Group {
    #[cfg(unix)]
    slot: Option<usize>,
    #[cfg(unix)]
    id: i32,
}

impl Group {
    /// The group `child` leads.
    pub(super) fn of(child: &Child) -> Group {
        #[cfg(unix)]
        {
            let id = i32::try_from(child.id()).unwrap_or(0);
            let slot = (id > 0)
                .then(|| {
                    unix::RUNNING.iter().position(|slot| {
                        slot.compare_exchange(
                            0,
                            id,
                            std::sync::atomic::Ordering::SeqCst,
                            std::sync::atomic::Ordering::SeqCst,
                        )
                        .is_ok()
                    })
                })
                .flatten();
            Group { slot, id }
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
            unix::kill_group(self.id);
        }
        let _ = child.kill();
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(slot) = self.slot {
            unix::RUNNING[slot].store(0, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// Kills the process groups of every program the oracle is running.
///
/// Async-signal-safe: it only reads atomics and calls `kill`, so a signal
/// handler may call it.
pub fn kill_running() {
    #[cfg(unix)]
    for slot in &unix::RUNNING {
        let id = slot.load(std::sync::atomic::Ordering::SeqCst);
        if id > 0 {
            unix::kill_group(id);
        }
    }
}

/// Makes `SIGINT`, `SIGTERM` and `SIGHUP` first [`kill_running`] and then
/// do what they would have done. A signal that is ignored stays ignored.
/// Does nothing outside Unix, where programs share the console's signals.
pub fn stop_running_on_signals() {
    #[cfg(unix)]
    unix::install();
}

#[cfg(unix)]
mod unix {
    use std::sync::atomic::AtomicI32;

    /// Process groups that may be running, `0` for a free slot. More
    /// programs at once than this go unlisted: a timeout still kills them,
    /// a signal does not.
    pub(super) static RUNNING: [AtomicI32; 1024] = [const { AtomicI32::new(0) }; 1024];

    pub(super) fn kill_group(id: i32) {
        // SAFETY: `kill` takes no pointers; a group that has gone is ESRCH.
        unsafe {
            libc::kill(-id, libc::SIGKILL);
        }
    }

    extern "C" fn on_signal(signal: libc::c_int) {
        super::kill_running();
        // SAFETY: `signal` and `raise` are async-signal-safe. With the
        // default action back, the raised signal ends the process as the
        // first one would have.
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
    }

    pub(super) fn install() {
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            // SAFETY: the handler is async-signal-safe (see `on_signal`).
            unsafe {
                let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
                if libc::signal(signal, handler) == libc::SIG_IGN {
                    libc::signal(signal, libc::SIG_IGN);
                }
            }
        }
    }
}
