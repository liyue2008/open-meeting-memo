//! macOS kernel peer identity and per-process NOTE_EXIT observation; never sends signals.
use crate::core::contracts::*;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use tokio::net::UnixStream;

pub(super) fn peer_pid(stream: &UnixStream) -> Result<libc::pid_t> {
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of_val(&pid) as libc::socklen_t;
    // LOCAL_PEERPID is kernel-provided, not a helper-controlled JSON PID.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&mut pid as *mut libc::pid_t).cast(),
            &mut len,
        )
    };
    if rc != 0 || len as usize != std::mem::size_of_val(&pid) || pid <= 0 {
        return Err(OmmError::unavailable(
            "capture peer process identity unavailable",
        ));
    }
    Ok(pid)
}

pub(super) struct ExitWatcher {
    fd: OwnedFd,
    pid: libc::pid_t,
}
impl ExitWatcher {
    pub(super) fn new(pid: libc::pid_t) -> Result<Self> {
        let raw = unsafe { libc::kqueue() };
        if raw < 0 {
            return Err(OmmError::unavailable(
                "capture process-exit observer unavailable",
            ));
        }
        // RAII closes on every error and on cancellation; CLOEXEC prevents child inheritance.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        if unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(OmmError::unavailable(
                "capture process-exit observer unavailable",
            ));
        }
        let change = libc::kevent {
            ident: pid as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        if unsafe { libc::kevent(raw, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) } < 0 {
            return Err(OmmError::unavailable(
                "capture process could not be observed; exit not confirmed",
            ));
        }
        Ok(Self { fd, pid })
    }
    pub(super) fn exited(&self) -> Result<bool> {
        let mut event = libc::kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let zero = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let n = unsafe {
            libc::kevent(
                self.fd.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut event,
                1,
                &zero,
            )
        };
        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        if n < 0
            || (n > 0
                && (event.flags & libc::EV_ERROR != 0
                    || event.ident != self.pid as libc::uintptr_t
                    || event.filter != libc::EVFILT_PROC
                    || event.fflags & libc::NOTE_EXIT == 0))
        {
            return Err(OmmError::new(
                ErrorCode::Backend,
                true,
                "capture process-exit observation failed",
            ));
        }
        Ok(n > 0)
    }
}
