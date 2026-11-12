//! Named sessions retain the live application, PTYs and emulator grids.
//! Only the window is hidden on detach; attaching wakes the existing event loop.
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::{
    fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    net::{UnixListener, UnixStream},
};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};

static NAME: OnceLock<String> = OnceLock::new();
static LOCK: OnceLock<File> = OnceLock::new();

pub fn enabled() -> bool {
    NAME.get().is_some()
}

/// Consume option operands without accidentally treating them as a command.
pub fn split_args(args: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut flags = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            return (flags, iter.collect());
        }
        if !arg.starts_with('-') {
            return (flags, std::iter::once(arg).chain(iter).collect());
        }
        let session = arg == "-S";
        flags.push(arg);
        if session {
            let _ = iter.next();
        }
    }
    (flags, Vec::new())
}

pub enum Launch {
    Attached,
    Start(Option<UnixListener>),
}

pub fn prepare() -> io::Result<Launch> {
    let mut args = std::env::args().skip(1);
    let mut name = None;
    while let Some(arg) = args.next() {
        if arg == "--" || !arg.starts_with('-') {
            break;
        }
        if arg == "-S" {
            let value = args
                .next()
                .ok_or_else(|| io::Error::other("-S requires a session name"))?;
            if name.is_some() || value.is_empty() || value.len() > 32 {
                return Err(io::Error::other("specify one session name of 1–32 bytes"));
            }
            name = Some(value);
        }
    }
    let Some(name) = name else {
        return Ok(Launch::Start(None));
    };
    if split_args(std::env::args().skip(1).collect())
        .0
        .iter()
        .any(|f| f == "-s" || f == "--standalone")
    {
        return Err(io::Error::other("-S cannot be combined with --standalone"));
    }
    let uid = unsafe { libc::geteuid() };
    let dir = std::path::PathBuf::from(format!("/tmp/mypixels-sessions-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(&dir)?;
    if !meta.is_dir() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::other("unsafe persistent session directory"));
    }
    let key: String = name.bytes().map(|b| format!("{b:02x}")).collect();
    let path = dir.join(format!("{key}.sock"));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(format!("{key}.lock")))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            match std::fs::remove_file(&path) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e),
            }
            let listener = UnixListener::bind(&path)?;
            NAME.set(name).unwrap();
            LOCK.set(lock).unwrap();
            return Ok(Launch::Start(Some(listener)));
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::WouldBlock {
            return Err(error);
        }
        if let Ok(mut stream) = UnixStream::connect(&path) {
            stream.set_read_timeout(Some(Duration::from_secs(10)))?;
            use std::io::Read;
            let mut ack = [0];
            stream.read_exact(&mut ack)?;
            return Ok(Launch::Attached);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other("session is not responding"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn serve(
    listener: UnixListener,
    tx: mpsc::Sender<crate::session::Request>,
    proxy: winit::event_loop::EventLoopProxy<()>,
) {
    std::thread::Builder::new()
        .name("persistent-attach".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                if tx.send(crate::session::Request::Raise).is_err() || proxy.send_event(()).is_err()
                {
                    break;
                }
                let _ = stream.write_all(&[1]);
            }
        })
        .expect("persistent session listener");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_operand_is_never_a_command() {
        let split = |a: &[&str]| split_args(a.iter().map(|s| s.to_string()).collect());
        assert_eq!(
            split(&["-S", "work", "-t", "sh", "-c", "echo hi"]),
            (
                vec!["-S".to_string(), "-t".to_string()],
                vec!["sh".to_string(), "-c".to_string(), "echo hi".to_string()]
            )
        );
        assert_eq!(
            split(&["-s", "sh", "-c"]),
            (vec!["-s".into()], vec!["sh".into(), "-c".into()])
        );
        assert_eq!(split(&["--", "-S"]), (vec![], vec!["-S".into()]));
    }
}
