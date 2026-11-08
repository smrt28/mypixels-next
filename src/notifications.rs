//! Terminal-originated desktop notifications (OSC 9, 99, and 777).
//! Parsing stays separate from the asynchronous session-bus worker.
use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use base64::Engine;
use futures::{FutureExt, StreamExt};

const TEXT_LIMIT: usize = 8192;
const PENDING_LIMIT: usize = 64;

#[derive(Clone, Debug)]
pub struct Raw {
    pub tab_id: usize,
    pub protocol: u16,
    pub data: String,
}

#[derive(Clone, Debug)]
pub struct Notification {
    pub tab_id: usize,
    pub id: String,
    pub title: String,
    pub body: String,
    pub focus: bool,
    pub report: bool,
    pub close_report: bool,
    pub urgency: u8,
    pub timeout: i32,
    pub silent: bool,
    occasion: Occasion,
}
impl Notification {
    fn new(tab_id: usize, id: String) -> Self {
        Self {
            tab_id,
            id,
            title: String::new(),
            body: String::new(),
            focus: true,
            report: false,
            close_report: false,
            urgency: 1,
            timeout: -1,
            silent: false,
            occasion: Occasion::Always,
        }
    }
    fn wire_id(&self) -> &str {
        if self.id.is_empty() { "0" } else { &self.id }
    }
}

#[derive(Clone, Copy, Debug)]
enum Occasion {
    Always,
    Unfocused,
    Invisible,
}

#[derive(Debug)]
pub enum Command {
    Show(Notification),
    Close { tab_id: usize, id: String },
    Alive { tab_id: usize, id: String },
    Forget(usize),
}

#[derive(Debug)]
pub enum Event {
    Activate {
        tab_id: usize,
        token: Option<String>,
    },
    Reply {
        tab_id: usize,
        text: String,
    },
}

#[derive(Default)]
pub struct Parser {
    pending: HashMap<(usize, String), Notification>,
}
impl Parser {
    pub fn forget(&mut self, tab_id: usize) {
        self.pending.retain(|(tab, _), _| *tab != tab_id);
    }

    /// Return a bus command or a protocol reply. The application's current
    /// focus/visibility decide OSC 99 occasions, not the PTY thread's window.
    pub fn parse(
        &mut self,
        raw: Raw,
        focused: bool,
        visible: bool,
    ) -> (Option<Command>, Option<String>) {
        let Raw {
            tab_id,
            protocol,
            data,
        } = raw;
        if data.len() > TEXT_LIMIT * 2 {
            return (None, None);
        }
        if protocol == 9 || protocol == 777 {
            let mut note = Notification::new(tab_id, String::new());
            if protocol == 9 {
                // OSC 9;4 is progress, OSC 9;9 is cwd, not a toast.
                if data.starts_with("4;") || data.starts_with("9;") {
                    return (None, None);
                }
                note.title = plain(&data);
            } else {
                let mut parts = data.splitn(3, ';');
                if parts.next() != Some("notify") {
                    return (None, None);
                }
                note.title = plain(parts.next().unwrap_or_default());
                note.body = plain(parts.next().unwrap_or_default());
            }
            return (finish(note, focused, visible), None);
        }
        if protocol != 99 {
            return (None, None);
        }
        let Some((metadata, payload)) = data.split_once(';') else {
            return (None, None);
        };
        let fields: HashMap<_, _> = metadata
            .split(':')
            .filter_map(|field| field.split_once('='))
            .collect();
        let id = fields.get("i").copied().unwrap_or_default();
        if id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_graphic() && !b":;=".contains(&b))
        {
            return (None, None);
        }
        let payload_type = fields.get("p").copied().unwrap_or("title");
        let wire_id = if id.is_empty() { "0" } else { id };
        match payload_type {
            "?" => {
                return (
                    None,
                    Some(format!(
                        "\x1b]99;i={wire_id}:p=?;a=focus,report:c=1:o=always,unfocused,invisible:p=title,body,?,close,alive\x1b\\"
                    )),
                );
            }
            "close" => {
                self.pending.remove(&(tab_id, id.to_string()));
                return (
                    (!id.is_empty()).then(|| Command::Close {
                        tab_id,
                        id: id.into(),
                    }),
                    None,
                );
            }
            "alive" => {
                return (
                    Some(Command::Alive {
                        tab_id,
                        id: wire_id.into(),
                    }),
                    None,
                );
            }
            "title" | "body" => {}
            _ => return (None, None),
        }
        let text = if fields.get("e") == Some(&"1") {
            let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(payload) else {
                return (None, None);
            };
            let Ok(text) = String::from_utf8(decoded) else {
                return (None, None);
            };
            plain(&text)
        } else {
            plain(payload)
        };
        let key = (tab_id, id.to_string());
        if !self.pending.contains_key(&key) && self.pending.len() >= PENDING_LIMIT {
            return (None, None);
        }
        let note = self
            .pending
            .entry(key.clone())
            .or_insert_with(|| Notification::new(tab_id, id.into()));
        if let Some(actions) = fields.get("a") {
            for action in actions.split(',') {
                match action {
                    "focus" => note.focus = true,
                    "-focus" => note.focus = false,
                    "report" => note.report = true,
                    "-report" => note.report = false,
                    _ => {}
                }
            }
        }
        if let Some(value) = fields.get("c") {
            note.close_report = *value == "1";
        }
        if let Some(value) = fields.get("o") {
            note.occasion = match *value {
                "unfocused" => Occasion::Unfocused,
                "invisible" => Occasion::Invisible,
                _ => Occasion::Always,
            };
        }
        if let Some(value) = fields
            .get("u")
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|value| *value <= 2)
        {
            note.urgency = value;
        }
        if let Some(value) = fields
            .get("w")
            .and_then(|value| value.parse::<i32>().ok())
            .filter(|value| *value >= -1)
        {
            note.timeout = value;
        }
        if let Some(value) = fields.get("s") {
            note.silent = *value == "c2lsZW50";
        }
        let target = if payload_type == "body" {
            &mut note.body
        } else {
            &mut note.title
        };
        if target.len() + text.len() > TEXT_LIMIT {
            self.pending.remove(&key);
            return (None, None);
        }
        target.push_str(&text);
        if fields.get("d") == Some(&"0") {
            return (None, None);
        }
        (
            finish(self.pending.remove(&key).unwrap(), focused, visible),
            None,
        )
    }
}

fn finish(mut note: Notification, focused: bool, visible: bool) -> Option<Command> {
    if matches!(note.occasion, Occasion::Unfocused) && focused
        || matches!(note.occasion, Occasion::Invisible) && visible
    {
        return None;
    }
    if note.title.is_empty() {
        note.title = std::mem::take(&mut note.body);
    }
    (!note.title.trim().is_empty()).then_some(Command::Show(note))
}

fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(TEXT_LIMIT)
        .collect()
}

fn markup_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub struct Desktop {
    commands: futures::channel::mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
}
impl Desktop {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (commands, rx) = futures::channel::mpsc::channel(64);
        let (tx, events) = mpsc::channel();
        std::thread::Builder::new()
            .name("desktop-notifications".into())
            .spawn(move || {
                if let Err(error) = futures::executor::block_on(serve(rx, tx, wake)) {
                    log::debug!("desktop notifications unavailable: {error}");
                }
            })
            .expect("notification worker");
        Self { commands, events }
    }
    pub fn send(&mut self, command: Command) {
        if let Err(error) = self.commands.try_send(command) {
            log::debug!("desktop notification dropped: {error}");
        }
    }
    pub fn events(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

async fn serve(
    mut commands: futures::channel::mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> zbus::Result<()> {
    let conn = zbus::connection::Builder::session()?
        .method_timeout(Duration::from_secs(3))
        .build()
        .await?;
    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await?;
    let mut signals = proxy.receive_all_signals().await?;
    let mut live: HashMap<u32, Notification> = HashMap::new();
    let mut tokens: HashMap<u32, String> = HashMap::new();
    let send = |event| {
        if events.send(event).is_ok() {
            wake();
        }
    };
    loop {
        futures::select! {
            command = commands.next().fuse() => {
                let Some(command) = command else { return Ok(()) };
                match command {
                    Command::Show(note) => {
                        let replaces = if note.id.is_empty() { 0 } else {
                            live.iter().find(|(_, old)| old.tab_id == note.tab_id && old.id == note.id).map_or(0, |(id, _)| *id)
                        };
                        let actions = if note.focus || note.report { vec!["default", "Show terminal"] } else { Vec::new() };
                        let mut hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
                        hints.insert("urgency", note.urgency.into());
                        hints.insert("desktop-entry", "mypixels".into());
                        if note.silent { hints.insert("suppress-sound", true.into()); }
                        let body = markup_escape(&note.body);
                        let result: zbus::Result<u32> = proxy.call("Notify", &("mypixels", replaces, "utilities-terminal", &note.title, &body, actions, hints, note.timeout)).await;
                        match result {
                            Ok(id) => {
                                tokens.remove(&id);
                                if live.len() >= 256 && !live.contains_key(&id) {
                                    if let Some(oldest) = live.keys().copied().min() { live.remove(&oldest); tokens.remove(&oldest); }
                                }
                                live.insert(id, note);
                            },
                            Err(error) => log::debug!("desktop notification: {error}"),
                        }
                    },
                    Command::Close { tab_id, id } => {
                        if let Some((&native_id, _)) = live.iter().find(|(_, note)| note.tab_id == tab_id && note.id == id) {
                            let _: zbus::Result<()> = proxy.call("CloseNotification", &(native_id,)).await;
                        }
                    },
                    Command::Alive { tab_id, id } => {
                        let mut ids: Vec<_> = live.values().filter(|note| note.tab_id == tab_id && !note.id.is_empty()).map(|note| note.id.as_str()).collect();
                        ids.sort_unstable();
                        send(Event::Reply { tab_id, text: format!("\x1b]99;i={id}:p=alive;{}\x1b\\", ids.join(",")) });
                    },
                    Command::Forget(tab_id) => {
                        let ids: Vec<_> = live.iter().filter(|(_, note)| note.tab_id == tab_id).map(|(id, _)| *id).collect();
                        for id in ids {
                            live.remove(&id);
                            tokens.remove(&id);
                            let _: zbus::Result<()> = proxy.call("CloseNotification", &(id,)).await;
                        }
                    },
                }
            },
            signal = signals.next().fuse() => {
                let Some(signal) = signal else { return Ok(()) };
                match signal.header().member().map(|member| member.as_str()) {
                    Some("ActivationToken") => {
                        if let Ok((id, token)) = signal.body().deserialize::<(u32, String)>() {
                            if live.contains_key(&id) { tokens.insert(id, token); }
                        }
                    },
                    Some("ActionInvoked") => {
                        if let Ok((id, action)) = signal.body().deserialize::<(u32, String)>() {
                            if action == "default" {
                                if let Some(note) = live.get(&id) {
                                    if note.focus { send(Event::Activate { tab_id: note.tab_id, token: tokens.remove(&id) }); }
                                    if note.report { send(Event::Reply { tab_id: note.tab_id, text: format!("\x1b]99;i={};\x1b\\", note.wire_id()) }); }
                                }
                            }
                        }
                    },
                    Some("NotificationClosed") => {
                        if let Ok((id, _)) = signal.body().deserialize::<(u32, u32)>() {
                            tokens.remove(&id);
                            if let Some(note) = live.remove(&id) {
                                if note.close_report { send(Event::Reply { tab_id: note.tab_id, text: format!("\x1b]99;i={}:p=close;\x1b\\", note.wire_id()) }); }
                            }
                        }
                    },
                    _ => {},
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw(tab_id: usize, protocol: u16, data: &str) -> Raw {
        Raw {
            tab_id,
            protocol,
            data: data.into(),
        }
    }
    #[test]
    fn legacy_notifications_preserve_semicolons_and_ignore_progress() {
        let mut parser = Parser::default();
        let (Some(Command::Show(note)), _) = parser.parse(
            raw(4, 9, "Approval requested: cargo test; echo done"),
            true,
            true,
        ) else {
            panic!()
        };
        assert_eq!(note.title, "Approval requested: cargo test; echo done");
        assert!(parser.parse(raw(4, 9, "4;1;50"), false, false).0.is_none());
        let (Some(Command::Show(note)), _) =
            parser.parse(raw(4, 777, "notify;Build;Passed; 10 tests"), false, false)
        else {
            panic!()
        };
        assert_eq!(
            (note.title.as_str(), note.body.as_str()),
            ("Build", "Passed; 10 tests")
        );
    }
    #[test]
    fn kitty_chunks_are_isolated_by_tab_and_support_encoded_bodies() {
        let mut parser = Parser::default();
        assert!(
            parser
                .parse(
                    raw(1, 99, "i=job:d=0:a=report;Approval requested"),
                    false,
                    false
                )
                .0
                .is_none()
        );
        parser.parse(raw(2, 99, "i=job:d=0;Other tab"), false, false);
        let (Some(Command::Show(note)), _) = parser.parse(
            raw(1, 99, "i=job:p=body:e=1;Y2FyZ28gdGVzdA=="),
            false,
            false,
        ) else {
            panic!()
        };
        assert_eq!(
            (note.tab_id, note.title.as_str(), note.body.as_str()),
            (1, "Approval requested", "cargo test")
        );
        assert!(note.focus && note.report);
        parser.forget(2);
        assert!(parser.pending.is_empty());
    }
    #[test]
    fn occasions_queries_and_malformed_payloads() {
        let mut parser = Parser::default();
        assert!(
            parser
                .parse(raw(1, 99, "o=unfocused;Hidden"), true, true)
                .0
                .is_none()
        );
        assert!(
            parser
                .parse(raw(1, 99, "o=invisible;Visible"), false, true)
                .0
                .is_none()
        );
        assert!(
            parser
                .parse(raw(1, 99, "e=1;invalid%%%"), false, false)
                .0
                .is_none()
        );
        let (_, reply) = parser.parse(raw(1, 99, "i=query:p=?;"), true, true);
        assert!(reply.unwrap().contains("p=title,body,?,close,alive"));
        assert!(
            parser
                .parse(raw(1, 99, "i=bad\x1b:p=?;"), false, false)
                .1
                .is_none()
        );
        assert_eq!(markup_escape("<b>A&B</b>"), "&lt;b&gt;A&amp;B&lt;/b&gt;");
    }
}

/// Click activation uses the compositor-provided token on Wayland. X11 can
/// focus the window directly. The borrowed display and surface remain owned
/// by the live winit Window throughout this call.
pub fn activate(window: &winit::window::Window, token: Option<&str>) {
    use winit::raw_window_handle::{
        HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    };
    if let (Ok(display), Ok(surface)) = (window.display_handle(), window.window_handle()) {
        if let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(surface)) =
            (display.as_raw(), surface.as_raw())
        {
            if let Some(token) = token {
                let result = wayland_activation(
                    display.display.as_ptr().cast(),
                    surface.surface.as_ptr().cast(),
                    token,
                );
                if result.is_ok() {
                    return;
                }
            }
            window.request_user_attention(Some(winit::window::UserAttentionType::Informational));
            return;
        }
    }
    window.focus_window();
}

fn wayland_activation(
    display: *mut std::ffi::c_void,
    surface: *mut std::ffi::c_void,
    token: &str,
) -> Result<(), String> {
    use wayland_client::protocol::{wl_registry, wl_surface};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
    #[derive(Default)]
    struct State {
        activation: Option<XdgActivationV1>,
    }
    impl Dispatch<wl_registry::WlRegistry, ()> for State {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name, interface, ..
            } = event
            {
                if interface == "xdg_activation_v1" {
                    state.activation = Some(registry.bind(name, 1, qh, ()));
                }
            }
        }
    }
    wayland_client::delegate_noop!(State: ignore XdgActivationV1);
    // SAFETY: called synchronously on the window thread with handles borrowed
    // from `window`. Guest mode never closes winit's display connection.
    let backend = unsafe { wayland_client::backend::Backend::from_foreign_display(display.cast()) };
    let conn = Connection::from_backend(backend);
    let mut queue = conn.new_event_queue::<State>();
    let registry = conn.display().get_registry(&queue.handle(), ());
    let mut state = State::default();
    queue
        .roundtrip(&mut state)
        .map_err(|error| error.to_string())?;
    let activation = state.activation.ok_or("xdg-activation unavailable")?;
    // SAFETY: this is the live wl_surface from the same borrowed display.
    let id = unsafe {
        wayland_client::backend::ObjectId::from_ptr(
            wl_surface::WlSurface::interface(),
            surface.cast(),
        )
    }
    .map_err(|error| error.to_string())?;
    let surface = wl_surface::WlSurface::from_id(&conn, id).map_err(|error| error.to_string())?;
    activation.activate(token.to_owned(), &surface);
    activation.destroy();
    drop(registry);
    conn.flush().map_err(|error| error.to_string())
}
