//! Notifications must cross the real parser, including arbitrary PTY chunking.
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Vec<Event>>>);
impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

#[test]
fn notifications_dispatch_once_without_rendering_text_or_ringing_bel() {
    let listener = Listener::default();
    let mut term = Term::new(Config::default(), &TermSize::new(40, 10), listener.clone());
    let mut parser: Processor = Processor::default();
    let bytes = b"\x1b]9;Approval requested: cargo test; echo done\x07\x1b]99;i=job:d=0;Build\x1b\\\x1b]99;i=job:p=body;Passed\x1b\\\x1b]777;notify;Tests;All passed\x07";
    for byte in bytes {
        parser.advance(&mut term, &[*byte]);
    }
    let events = listener.0.lock().unwrap();
    let notes: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            Event::Notification { protocol, data } => Some((*protocol, data.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        notes,
        vec![
            (9, "Approval requested: cargo test; echo done"),
            (99, "i=job:d=0;Build"),
            (99, "i=job:p=body;Passed"),
            (777, "notify;Tests;All passed")
        ]
    );
    assert!(!events.iter().any(|event| matches!(event, Event::Bell)));
    assert_eq!(term.grid().cursor.point.column.0, 0);
}
