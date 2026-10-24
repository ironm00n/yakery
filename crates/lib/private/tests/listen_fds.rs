#![allow(unsafe_code)]

use std::{
    env, fs,
    os::{
        fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd},
        unix::net::{UnixListener, UnixStream},
    },
    thread,
    time::Duration,
};

use rustix::io::{FdFlags, dup2, fcntl_getfd, fcntl_setfd};

// One thread, as listen_fds requires, so no test harness.
fn main() {
    assert!(private::listen_fds().unwrap().is_empty());
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("api.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    // SAFETY: fd 3 is ours to take: nothing else in this process opened it, and dup2 puts the listener there.
    let mut at3 = unsafe { OwnedFd::from_raw_fd(3) };
    if listener.as_raw_fd() != 3 {
        dup2(&listener, &mut at3).unwrap();
        drop(listener);
    } else {
        let _ = listener.into_raw_fd();
    }
    fcntl_setfd(&at3, FdFlags::empty()).unwrap();
    let _ = at3.into_raw_fd();
    let set = |pid: u32| {
        // SAFETY: no other thread runs yet.
        unsafe {
            env::set_var("LISTEN_PID", pid.to_string());
            env::set_var("LISTEN_FDS", "1");
            env::set_var("LISTEN_FDNAMES", "api");
        }
    };

    set(std::process::id());
    let sleeper = thread::spawn(|| thread::sleep(Duration::from_millis(300)));
    let refused = private::listen_fds().unwrap_err().to_string();
    assert!(refused.contains("before a second thread starts"), "{refused}");
    assert_eq!(env::var("LISTEN_FDS").as_deref(), Ok("1"));
    sleeper.join().unwrap();
    // a joined thread can stay listed in /proc for a moment after it is gone
    let gone = (0..5000).any(|_| {
        thread::sleep(Duration::from_millis(1));
        fs::read_dir("/proc/self/task").unwrap().count() == 1
    });
    assert!(gone, "the joined thread is still in /proc/self/task");

    let passed = private::listen_fds().unwrap();
    assert_eq!(
        passed.iter().map(|(n, fd)| (n.as_str(), fd.as_raw_fd())).collect::<Vec<_>>(),
        [("api", 3)]
    );
    assert!(
        ["LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES"].iter().all(|v| env::var_os(v).is_none())
    );
    // SAFETY: fd 3 is open; the borrow ends before it closes.
    assert!(fcntl_getfd(unsafe { BorrowedFd::borrow_raw(3) }).unwrap().contains(FdFlags::CLOEXEC));
    let listener = UnixListener::from(passed.into_iter().next().unwrap().1);
    let _client = UnixStream::connect(&socket).unwrap();
    listener.accept().unwrap();
}
