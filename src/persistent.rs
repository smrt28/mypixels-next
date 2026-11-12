//! Named sessions retain live PTYs and emulator grids independently of windows.
//! The registry lock serializes creation, attachment reservations and detachment.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::{
    fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    net::{UnixListener, UnixStream},
};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

static STATE: OnceLock<PathBuf> = OnceLock::new();
static LOCK: OnceLock<File> = OnceLock::new();

pub fn enabled() -> bool {
    STATE.get().is_some()
}

// This descriptor exists only across the private re-exec. It is removed from
// the environment and marked close-on-exec before any terminal can inherit it.
const STARTUP_FD: &str = "_MYPIXELS_STARTUP_FD";
static STARTUP: OnceLock<Mutex<Option<UnixStream>>> = OnceLock::new();

pub fn initialize_background() -> io::Result<()> {
    let Some(value) = std::env::var_os(STARTUP_FD) else {
        return Ok(());
    };
    // Called at the start of main, before threads or terminal children exist.
    unsafe {
        std::env::remove_var(STARTUP_FD);
    }
    let fd: i32 = value
        .to_str()
        .and_then(|v| v.parse().ok())
        .filter(|fd| *fd >= 3)
        .ok_or_else(|| io::Error::other("invalid startup descriptor"))?;
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    STARTUP.set(Mutex::new(Some(stream))).unwrap();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        startup_failed(&info.to_string());
        previous(info);
    }));
    Ok(())
}

fn startup_result(result: Result<(), String>) {
    let Some(startup) = STARTUP.get() else { return };
    let Some(mut stream) = startup.lock().unwrap().take() else {
        return;
    };
    if let Ok(mut message) = serde_json::to_vec(&result) {
        message.push(b'\n');
        let _ = stream.write_all(&message);
    }
}

pub fn startup_ready() {
    startup_result(Ok(()));
}
pub fn startup_failed(message: &str) {
    startup_result(Err(message.into()));
}

/// Re-exec rather than fork an initialized renderer or a multithreaded app.
/// The foreground launcher owns no session lock; the child performs the same
/// atomic selection as before and reports readiness or startup errors back.
pub fn launch_background(flags: &[String]) -> io::Result<bool> {
    if STARTUP.get().is_some() || !flags.iter().any(|f| f == "-S" || f == "-A") {
        return Ok(false);
    }
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.args(std::env::args_os().skip(1));
    let mut child = spawn_background(command)?;
    // An attaching child exits after the handshake; a newly created session
    // stays alive. Reap the former when it has already exited, without waiting
    // for the lifetime of the latter.
    let _ = child.try_wait();
    Ok(true)
}

fn spawn_background(mut command: std::process::Command) -> io::Result<std::process::Child> {
    use std::io::BufRead;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    let (parent, child_socket) = UnixStream::pair()?;
    parent.set_read_timeout(Some(Duration::from_secs(60)))?;
    let fd = child_socket.as_raw_fd();
    command
        .env(STARTUP_FD, fd.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Only async-signal-safe syscalls run between fork and exec. Keep the
    // handshake socket across exec; all other Rust-owned descriptors retain
    // their normal close-on-exec behavior.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() == -1 || libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    drop(child_socket);
    let response = (|| -> io::Result<()> {
        let mut message = String::new();
        std::io::BufReader::new(parent).read_line(&mut message)?;
        if message.is_empty() {
            return Err(io::Error::other(
                "background session exited before opening a window",
            ));
        }
        let result: Result<(), String> = serde_json::from_str(&message)?;
        result.map_err(io::Error::other)
    })();
    if let Err(error) = response {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok(child)
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

enum Prepared {
    Attached,
    New {
        listener: UnixListener,
        lock: File,
        state: PathBuf,
    },
}

#[derive(Clone, Debug)]
struct Status {
    name: String,
    attached: bool,
    pending: u64,
}

fn read_status(path: &Path) -> io::Result<Status> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(Status {
        name: value["name"]
            .as_str()
            .ok_or_else(|| io::Error::other("invalid session name"))?
            .into(),
        attached: value["attached"]
            .as_bool()
            .ok_or_else(|| io::Error::other("invalid session status"))?,
        pending: value["pending"].as_u64().unwrap_or(0),
    })
}

fn write_status(path: &Path, status: &Status) -> io::Result<()> {
    std::fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({
            "name": status.name, "attached": status.attached, "pending": status.pending,
        }))?,
    )
}

fn open_lock(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
}

fn try_lock(file: &File) -> io::Result<bool> {
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(error)
    }
}

fn registry_lock(dir: &Path) -> io::Result<File> {
    let file = open_lock(&dir.join("registry.lock"))?;
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(file);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

// Canonical positive decimal names only; string comparison avoids overflow for
// explicitly named sessions with numbers larger than a machine integer.
fn number(name: &str) -> Option<&str> {
    let n = name.strip_prefix('#')?;
    (!n.is_empty() && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())).then_some(n)
}

fn numeric_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

fn decode_key(key: &str) -> Option<String> {
    if key.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..key.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(key.get(i..i + 2)?, 16).ok())
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}

/// Called with the registry lock held. A held lifetime lock, rather than a
/// stale socket or status file, defines whether a session actually exists.
fn live_sessions(dir: &Path) -> io::Result<Vec<(String, String, bool)>> {
    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let filename = entry.file_name();
        let Some(key) = filename.to_str().and_then(|s| s.strip_suffix(".lock")) else {
            continue;
        };
        if key == "registry" {
            continue;
        }
        let lock = open_lock(&entry.path())?;
        if try_lock(&lock)? {
            continue;
        }
        let status = read_status(&dir.join(format!("{key}.state")));
        match status {
            Ok(status) => sessions.push((
                key.into(),
                status.name,
                status.attached || status.pending > 0,
            )),
            // Older sessions have no attachment metadata: reserve their names
            // and conservatively treat them as attached.
            Err(_) => {
                if let Some(name) = decode_key(key) {
                    sessions.push((key.into(), name, true));
                }
            }
        }
    }
    Ok(sessions)
}

/// Read a consistent snapshot without attaching to or reserving any session.
fn list_in(dir: &Path) -> io::Result<Vec<(String, bool)>> {
    let _registry = registry_lock(dir)?;
    let mut sessions: Vec<_> = live_sessions(dir)?
        .into_iter()
        .map(|(_, name, attached)| (name, attached))
        .collect();
    sessions.sort_by(|a, b| match (number(&a.0), number(&b.0)) {
        (Some(a), Some(b)) => numeric_cmp(a, b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });
    Ok(sessions)
}

fn validate_directory(dir: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.permissions().mode() & 0o077 != 0
    {
        return Err(io::Error::other("unsafe persistent session directory"));
    }
    Ok(())
}

pub fn list() -> io::Result<String> {
    let uid = unsafe { libc::geteuid() };
    let dir = PathBuf::from(format!("/tmp/mypixels-sessions-{uid}"));
    match validate_directory(&dir) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok("No persistent sessions.\n".into());
        }
        Err(e) => return Err(e),
    }
    let sessions = list_in(&dir)?;
    if sessions.is_empty() {
        return Ok("No persistent sessions.\n".into());
    }
    Ok(sessions
        .into_iter()
        .map(|(name, attached)| {
            format!(
                "{}\t{}\n",
                name.escape_debug(),
                if attached { "attached" } else { "detached" }
            )
        })
        .collect())
}

fn auto_name(sessions: &[(String, String, bool)]) -> String {
    if let Some((_, name, _)) = sessions
        .iter()
        .filter(|(_, name, attached)| !attached && number(name).is_some())
        .min_by(|a, b| numeric_cmp(number(&a.1).unwrap(), number(&b.1).unwrap()))
    {
        return name.clone();
    }
    let used: std::collections::HashSet<&str> = sessions
        .iter()
        .filter_map(|(_, name, _)| number(name))
        .collect();
    let mut n = 1u64;
    while used.contains(n.to_string().as_str()) {
        n += 1;
    }
    format!("#{n}")
}

fn acquire(dir: &Path, explicit: Option<&str>) -> io::Result<Prepared> {
    let _registry = registry_lock(dir)?;
    let sessions = live_sessions(dir)?;
    let name = explicit
        .map(str::to_string)
        .unwrap_or_else(|| auto_name(&sessions));
    let key = if let Some((key, _, _)) = sessions.iter().find(|(_, n, _)| *n == name) {
        key.clone()
    } else if name.len() <= 32 {
        name.bytes().map(|b| format!("{b:02x}")).collect()
    } else {
        // Long arbitrary names cannot be embedded in Unix socket paths. Allocate
        // a short key under the same registry lock and keep the full name in state.
        let mut index = 1u64;
        while sessions
            .iter()
            .any(|(key, _, _)| *key == format!("long-{index}"))
        {
            index += 1;
        }
        format!("long-{index}")
    };
    let path = dir.join(format!("{key}.sock"));
    let state = dir.join(format!("{key}.state"));
    let lock = open_lock(&dir.join(format!("{key}.lock")))?;
    if try_lock(&lock)? {
        match std::fs::remove_file(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(path)?;
        write_status(
            &state,
            &Status {
                name,
                attached: true,
                pending: 0,
            },
        )?;
        return Ok(Prepared::New {
            listener,
            lock,
            state,
        });
    }
    let mut status = read_status(&state).unwrap_or(Status {
        name,
        attached: true,
        pending: 0,
    });
    // Reserve before releasing the registry so another -A cannot select us.
    status.pending += 1;
    write_status(&state, &status)?;
    // The reservation makes selection atomic while allowing the UI to take
    // the registry lock and finish restoring windows before acknowledging.
    drop(_registry);
    let result = attach(&path);
    if result.is_err() {
        let _registry = registry_lock(dir)?;
        let mut status = read_status(&state)?;
        status.pending = status.pending.saturating_sub(1);
        write_status(&state, &status)?;
    }
    result.map(|()| Prepared::Attached)
}

fn attach(path: &Path) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(mut stream) = UnixStream::connect(path) {
            stream.set_read_timeout(Some(Duration::from_secs(35)))?;
            let mut ack = [0];
            stream.read_exact(&mut ack)?;
            return if ack == [1] {
                Ok(())
            } else {
                Err(io::Error::other("invalid session reply"))
            };
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other("session is not responding"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn prepare() -> io::Result<Launch> {
    let mut args = std::env::args().skip(1);
    let mut name = None;
    let mut automatic = false;
    while let Some(arg) = args.next() {
        if arg == "--" || !arg.starts_with('-') {
            break;
        }
        match arg.as_str() {
            "-A" => automatic = true,
            "-S" => {
                let value = args
                    .next()
                    .ok_or_else(|| io::Error::other("-S requires a session name"))?;
                if name.is_some() || value.is_empty() {
                    return Err(io::Error::other("specify one nonempty session name"));
                }
                name = Some(value);
            }
            _ => (),
        }
    }
    if automatic && name.is_some() {
        return Err(io::Error::other("-A and -S are mutually exclusive"));
    }
    if !automatic && name.is_none() {
        return Ok(Launch::Start(None));
    }
    if split_args(std::env::args().skip(1).collect())
        .0
        .iter()
        .any(|f| f == "-s" || f == "--standalone")
    {
        return Err(io::Error::other(
            "persistent sessions cannot be combined with --standalone",
        ));
    }
    let uid = unsafe { libc::geteuid() };
    let dir = PathBuf::from(format!("/tmp/mypixels-sessions-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e),
    }
    validate_directory(&dir)?;
    match acquire(&dir, name.as_deref())? {
        Prepared::Attached => Ok(Launch::Attached),
        Prepared::New {
            listener,
            lock,
            state,
        } => {
            STATE.set(state).unwrap();
            LOCK.set(lock).unwrap();
            Ok(Launch::Start(Some(listener)))
        }
    }
}

/// A session is detached only when all its windows have been closed. Pending
/// attaches remain reserved even if a close event races a queued Raise event.
pub fn set_attached(attached: bool, complete_attach: bool) {
    let Some(path) = STATE.get() else { return };
    if let Err(e) = update_status(path, attached, complete_attach) {
        log::error!("cannot update persistent session status: {e}");
    }
}

fn update_status(path: &Path, attached: bool, complete_attach: bool) -> io::Result<()> {
    let _registry = registry_lock(path.parent().unwrap())?;
    let mut status = read_status(path)?;
    status.attached = attached;
    if complete_attach {
        status.pending = status.pending.saturating_sub(1);
    }
    write_status(path, &status)
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
                if let Err(e) = request_attach(&mut stream, &tx, || proxy.send_event(()).is_ok()) {
                    log::debug!("persistent attachment failed: {e}");
                }
            }
        })
        .expect("persistent session listener");
}

fn request_attach(
    stream: &mut UnixStream,
    tx: &mpsc::Sender<crate::session::Request>,
    wake: impl FnOnce() -> bool,
) -> io::Result<()> {
    let (respond, ready) = mpsc::channel();
    tx.send(crate::session::Request::PersistentRaise { respond })
        .map_err(|_| io::Error::other("session has exited"))?;
    if !wake() {
        return Err(io::Error::other("session has exited"));
    }
    // A launcher returns after the app restores its windows, not merely
    // after a request has been queued. No registry lock is held here.
    ready
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| io::Error::other("session did not restore its windows"))?;
    stream.write_all(&[1])
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

    struct Registry(PathBuf);
    impl Registry {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = PathBuf::from(format!(
                "/tmp/mp-auto-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn create(&self, name: Option<&str>) -> (UnixListener, File, PathBuf) {
            match acquire(&self.0, name).unwrap() {
                Prepared::New {
                    listener,
                    lock,
                    state,
                } => (listener, lock, state),
                Prepared::Attached => panic!("expected creation"),
            }
        }
    }
    impl Drop for Registry {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn ack(listener: &UnixListener) -> std::thread::JoinHandle<()> {
        let listener = listener.try_clone().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(&[1]).unwrap();
        })
    }

    #[test]
    fn automatic_selection_matches_the_example_and_reserves_attachments() {
        let dir = Registry::new();
        let _one = dir.create(Some("#1"));
        let (listener, _two_lock, two_state) = dir.create(Some("#2"));
        let _four = dir.create(Some("#4"));
        let _work = dir.create(Some("work"));
        update_status(&two_state, false, false).unwrap();
        let server = ack(&listener);
        assert!(matches!(acquire(&dir.0, None).unwrap(), Prepared::Attached));
        server.join().unwrap();
        assert_eq!(read_status(&two_state).unwrap().pending, 1);
        // A close event before the queued reattach must not clear its reservation.
        update_status(&two_state, false, false).unwrap();
        let (_, _three_lock, three_state) = dir.create(None);
        assert_eq!(read_status(&three_state).unwrap().name, "#3");
        update_status(&two_state, true, true).unwrap();
        assert_eq!(read_status(&two_state).unwrap().pending, 0);
        let (_, _five_lock, five_state) = dir.create(None);
        assert_eq!(read_status(&five_state).unwrap().name, "#5");
        // Completing and then detaching makes exactly the same session reusable.
        update_status(&two_state, false, false).unwrap();
        let server = ack(&listener);
        assert!(matches!(acquire(&dir.0, None).unwrap(), Prepared::Attached));
        server.join().unwrap();
    }

    #[test]
    fn concurrent_creation_allocates_distinct_lowest_numbers() {
        let dir = Registry::new();
        let barrier = std::sync::Barrier::new(8);
        let sessions = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        dir.create(None)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        let mut names: Vec<_> = sessions
            .iter()
            .map(|(_, _, path)| read_status(path).unwrap().name)
            .collect();
        names.sort();
        assert_eq!(names, (1..=8).map(|n| format!("#{n}")).collect::<Vec<_>>());
    }

    #[test]
    fn concurrent_launches_claim_a_detached_session_only_once() {
        let dir = Registry::new();
        let (listener, _lock, state) = dir.create(Some("#1"));
        update_status(&state, false, false).unwrap();
        let server = ack(&listener);
        let barrier = std::sync::Barrier::new(8);
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        acquire(&dir.0, None).unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        server.join().unwrap();
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Prepared::Attached))
                .count(),
            1
        );
        let mut created: Vec<_> = results
            .iter()
            .filter_map(|r| match r {
                Prepared::New { state, .. } => Some(read_status(state).unwrap().name),
                _ => None,
            })
            .collect();
        created.sort();
        assert_eq!(
            created,
            (2..=8).map(|n| format!("#{n}")).collect::<Vec<_>>()
        );
    }

    #[test]
    fn failed_attach_releases_the_selection_reservation() {
        let dir = Registry::new();
        let (listener, _lock, state) = dir.create(Some("#1"));
        update_status(&state, false, false).unwrap();
        let socket = listener.try_clone().unwrap();
        let server = std::thread::spawn(move || {
            drop(socket.accept().unwrap());
        });
        assert!(acquire(&dir.0, None).is_err());
        server.join().unwrap();
        let status = read_status(&state).unwrap();
        assert!(!status.attached);
        assert_eq!(status.pending, 0);
        let server = ack(&listener);
        assert!(matches!(acquire(&dir.0, None).unwrap(), Prepared::Attached));
        server.join().unwrap();
    }

    #[test]
    fn listing_shows_live_statuses_in_numeric_order_without_reserving() {
        let dir = Registry::new();
        assert!(list_in(&dir.0).unwrap().is_empty());
        let _ten = dir.create(Some("#10"));
        let (_, _two_lock, two_state) = dir.create(Some("#2"));
        let _work = dir.create(Some("work"));
        let dead = dir.create(Some("gone"));
        drop(dead);
        update_status(&two_state, false, false).unwrap();
        let before = std::fs::read(&two_state).unwrap();
        assert_eq!(
            list_in(&dir.0).unwrap(),
            vec![
                ("#2".into(), false),
                ("#10".into(), true),
                ("work".into(), true),
            ]
        );
        assert_eq!(std::fs::read(&two_state).unwrap(), before);
        let _registry = registry_lock(&dir.0).unwrap();
        let mut status = read_status(&two_state).unwrap();
        status.pending = 1;
        write_status(&two_state, &status).unwrap();
        drop(_registry);
        assert_eq!(list_in(&dir.0).unwrap()[0], ("#2".into(), true));
    }

    #[test]
    fn stale_records_and_sockets_do_not_reserve_numbers() {
        let dir = Registry::new();
        let first = dir.create(None);
        let state = first.2.clone();
        drop(first);
        assert!(state.exists());
        let (_, _lock, state) = dir.create(None);
        assert_eq!(read_status(&state).unwrap().name, "#1");
    }

    #[test]
    fn long_arbitrary_names_reconnect_without_path_limits() {
        let dir = Registry::new();
        let name = format!("project/{} ☃", "work".repeat(100));
        let (listener, _lock, state) = dir.create(Some(&name));
        assert_eq!(read_status(&state).unwrap().name, name);
        let server = ack(&listener);
        assert!(matches!(
            acquire(&dir.0, Some(&name)).unwrap(),
            Prepared::Attached
        ));
        server.join().unwrap();
        let (_, _other_lock, other_state) = dir.create(Some(&(name + "other")));
        assert_ne!(other_state, state);
    }

    struct BackgroundChild(std::process::Child);
    impl Drop for BackgroundChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn background_process_is_ready_detached_and_independent_of_standard_io() {
        let dir = Registry::new();
        let info = dir.0.join("background.json");
        let heartbeat = dir.0.join("heartbeat");
        let mut command = std::process::Command::new("python3");
        command
            .arg("-c")
            .arg(
                r#"
import json, os, sys, time
from pathlib import Path
Path(sys.argv[1]).write_text(json.dumps(dict(
    pid=os.getpid(), sid=os.getsid(0), pgid=os.getpgrp(), cwd=os.getcwd(),
    stdio=[os.readlink('/proc/self/fd/' + str(i)) for i in range(3)])))
os.write(int(os.environ['_MYPIXELS_STARTUP_FD']), b'{"Ok":null}\n')
while True:
    Path(sys.argv[2]).write_text(str(time.monotonic_ns()))
    time.sleep(.02)
"#,
            )
            .arg(&info)
            .arg(&heartbeat)
            .current_dir(&dir.0);
        let mut child = BackgroundChild(spawn_background(command).unwrap());
        assert!(child.0.try_wait().unwrap().is_none());
        let details: serde_json::Value =
            serde_json::from_slice(&std::fs::read(info).unwrap()).unwrap();
        assert_eq!(details["pid"], child.0.id());
        assert_eq!(details["pid"], details["sid"]);
        assert_eq!(details["pid"], details["pgid"]);
        assert_eq!(details["cwd"], dir.0.to_str().unwrap());
        assert_eq!(
            details["stdio"],
            serde_json::json!(["/dev/null", "/dev/null", "/dev/null"])
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while !heartbeat.exists() {
            assert!(
                Instant::now() < deadline,
                "background process stopped after readiness"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn background_startup_failures_and_premature_exit_reach_the_launcher() {
        let mut command = std::process::Command::new("python3");
        command.arg("-c").arg(
            r#"
import os, time
os.write(int(os.environ['_MYPIXELS_STARTUP_FD']), b'{"Err":"test startup failure"}\n')
time.sleep(30)
"#,
        );
        let error = spawn_background(command).unwrap_err();
        assert!(error.to_string().contains("test startup failure"));
        let mut command = std::process::Command::new("python3");
        command.args(["-c", "raise SystemExit(7)"]);
        assert!(
            spawn_background(command)
                .unwrap_err()
                .to_string()
                .contains("before opening a window")
        );
    }

    #[test]
    fn attach_acknowledgement_waits_for_window_restoration_without_holding_registry() {
        let dir = Registry::new();
        let (listener, _lock, state) = dir.create(Some("#1"));
        update_status(&state, false, false).unwrap();
        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            request_attach(&mut stream, &tx, || true).unwrap();
        });
        let registry = dir.0.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let client = std::thread::spawn(move || {
            let result = acquire(&registry, None);
            done_tx.send(()).unwrap();
            result
        });
        let crate::session::Request::PersistentRaise { respond } =
            rx.recv_timeout(Duration::from_secs(2)).unwrap()
        else {
            panic!("wrong request")
        };
        assert!(
            done_rx.try_recv().is_err(),
            "attach returned before restoring windows"
        );
        assert_eq!(read_status(&state).unwrap().pending, 1);
        update_status(&state, true, true).unwrap();
        respond.send(()).unwrap();
        assert!(matches!(
            client.join().unwrap().unwrap(),
            Prepared::Attached
        ));
        server.join().unwrap();
        assert!(read_status(&state).unwrap().attached);
        assert_eq!(read_status(&state).unwrap().pending, 0);
    }

    #[test]
    fn automatic_names_sort_numerically_and_ignore_other_names() {
        let entries = |names: &[&str]| {
            names
                .iter()
                .map(|n| (String::new(), n.to_string(), false))
                .collect::<Vec<_>>()
        };
        assert_eq!(auto_name(&entries(&["#10", "#2", "work"])), "#2");
        assert_eq!(
            auto_name(&entries(&["#0", "#01", "#-1", "#", "#１", "work"])),
            "#1"
        );
        assert_eq!(
            auto_name(&entries(&["#18446744073709551616", "#20"])),
            "#20"
        );
        assert_eq!(
            auto_name(&[(String::new(), "#18446744073709551616".into(), true)]),
            "#1"
        );
    }

    #[test]
    fn automatic_flag_does_not_consume_the_terminal_command() {
        assert_eq!(
            split_args(vec!["-A".into(), "sh".into(), "-S".into()]),
            (vec!["-A".into()], vec!["sh".into(), "-S".into()])
        );
    }
}
