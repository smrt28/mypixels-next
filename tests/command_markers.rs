//! Command input must be copied in the parser, before a fast command can
//! overwrite the grid, and must use cell positions rather than byte offsets.
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{CommandOutput, Event, EventListener};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

#[derive(Clone, Default)]
struct Marks(Arc<Mutex<Vec<(u8, Option<i32>, Option<String>)>>>);

impl EventListener for Marks {
    fn send_event(&self, event: Event) {
        if let Event::Semantic { code, exit, command, .. } = event {
            self.0.lock().unwrap().push((code, exit, command));
        }
    }
}

fn commands(bytes: &[u8], columns: usize) -> Vec<(u8, Option<i32>, Option<String>)> {
    let marks = Marks::default();
    let mut term = Term::new(Config::default(), &TermSize::new(columns, 8), marks.clone());
    let mut parser: Processor = Processor::default();
    // A PTY can split markers or Unicode characters across any read boundary.
    for byte in bytes {
        parser.advance(&mut term, &[*byte]);
    }
    let result = marks.0.lock().unwrap().clone();
    result
}

#[test]
fn input_is_captured_before_output_clears_the_screen() {
    let stream = concat!(
        "\x1b]133;A\x07project\r\n界 ❯ \x1b]133;B\x07",
        "printf 'héllo'\r\n\x1b]133;C\x07",
        "\x1b[2J\x1b[Hhéllo\r\n\x1b]133;D;0\x07",
        "\x1b]133;A\x07next prompt\x1b]133;B\x07",
    );
    let marks = commands(stream.as_bytes(), 80);
    assert_eq!(marks.iter().filter(|m| m.0 == 2).count(), 1);
    assert_eq!(marks.iter().find(|m| m.0 == 2).unwrap().2.as_deref(), Some("printf 'héllo'"));
    assert_eq!(marks.iter().find(|m| m.0 == 3).unwrap().1, Some(0));
    assert!(marks.last().unwrap().2.is_none(), "an idle prompt has no command");
}

#[test]
fn wrapped_input_and_wide_characters_keep_the_whole_command() {
    let command = "printf '%s' '界 e\u{301} abcdefghijklmnopqrstuvwxyz'";
    let stream = format!("\x1b]133;A\x07λ> \x1b]133;B\x07{command}\r\n\x1b]133;C\x07\x1b]133;D;7\x07");
    let marks = commands(stream.as_bytes(), 20);
    assert_eq!(marks.iter().find(|m| m.0 == 2).unwrap().2.as_deref(), Some(command));
    assert_eq!(marks.iter().find(|m| m.0 == 3).unwrap().1, Some(7));
}

#[test]
fn command_can_end_on_the_same_row_as_the_execution_marker() {
    let marks = commands(b"\x1b]133;A\x07$ \x1b]133;B\x07echo ok\x1b]133;C\x07", 80);
    assert_eq!(marks.last().unwrap().2.as_deref(), Some("echo ok"));
}

#[test]
fn stale_or_missing_input_anchors_do_not_capture_unrelated_text() {
    let stream = b"\x1b]133;A\x07$ \x1b]133;B\x07old input\r\n\x1b]133;D;0\x07\x1b]133;C\x07";
    assert!(commands(stream, 80).last().unwrap().2.is_none());
}

#[test]
fn empty_input_is_distinct_from_an_unavailable_command() {
    for input in ["", " ", "   \t", "\u{2003}"] {
        for newline in ["", "\r\n"] {
            let stream = format!("\x1b]133;A\x07$ \x1b]133;B\x07{input}{newline}\x1b]133;C\x07");
            let marks = commands(stream.as_bytes(), 80);
            assert_eq!(marks.last().unwrap().2.as_deref(), Some(""), "input={input:?}, newline={newline:?}");
        }
    }
}

#[derive(Clone, Default)]
struct Outputs(Arc<Mutex<Vec<(Option<CommandOutput>, bool)>>>);

impl EventListener for Outputs {
    fn send_event(&self, event: Event) {
        if let Event::Semantic { code: 3, output, alt, .. } = event {
            self.0.lock().unwrap().push((output, alt));
        }
    }
}

fn outputs(stream: &str, columns: usize, history: usize) -> Vec<(Option<CommandOutput>, bool)> {
    let marks = Outputs::default();
    // Test both a single PTY read and arbitrary read boundaries.
    for chunk_size in [stream.len(), 1] {
        let config = Config { scrolling_history: history, ..Config::default() };
        let mut term = Term::new(config, &TermSize::new(columns, 8), marks.clone());
        let mut parser: Processor = Processor::default();
        for chunk in stream.as_bytes().chunks(chunk_size) {
            parser.advance(&mut term, chunk);
        }
    }
    let result = marks.0.lock().unwrap().clone();
    result
}

#[test]
fn output_excludes_multiline_prompts_and_wrapped_command_input() {
    let stream = concat!(
        "\x1b]133;A\x07界 project\r\n❯ \x1b]133;B\x07",
        "printf '%s' 'a very long command that wraps over several rows'\r\n",
        "\x1b]133;C\x07héllo 界 e\u{301}\r\nsecond output line\r\n",
        "\x1b]133;D;0\x07\x1b]133;A\x07next project\r\n❯ \x1b]133;B\x07",
    );
    for (output, alt) in outputs(stream, 20, 100) {
        let output = output.unwrap();
        assert_eq!(output.text, "héllo 界 e\u{301}\nsecond output line");
        assert!(!output.truncated_head);
        assert!(!alt);
    }
}

#[test]
fn partial_line_output_is_saved_before_the_next_prompt() {
    for (result, suffix) in [("", ""), ("hello", ""), ("hello", "\r\n")] {
        let stream = format!(
            "\x1b]133;A\x07$ \x1b]133;B\x07command\x1b]133;C\x07{result}{suffix}\x1b]133;D;0\x07\x1b]133;A\x07NEXT PROMPT $ "
        );
        for (output, _) in outputs(&stream, 80, 100) {
            assert_eq!(output.unwrap().text, result);
        }
    }
}

#[test]
fn output_keeps_the_final_cell_at_a_pending_wrap() {
    let stream = "\x1b]133;A\x07$ \x1b]133;B\x07command\r\n\x1b]133;C\x071234567890\x1b]133;D;0\x07";
    for (output, _) in outputs(stream, 10, 100) {
        assert_eq!(output.unwrap().text, "1234567890");
    }
}

#[test]
fn multiple_commands_in_one_read_have_separate_output() {
    let stream = concat!(
        "\x1b]133;A\x07$ \x1b]133;B\x07first\r\n\x1b]133;C\x07one\r\n\x1b]133;D;0\x07",
        "\x1b]133;A\x07$ \x1b]133;B\x07second\r\n\x1b]133;C\x07two\x1b]133;D;0\x07",
        "\x1b]133;A\x07$ \x1b]133;B\x07",
    );
    let outputs = outputs(stream, 80, 100);
    let text: Vec<_> = outputs.iter().map(|(o, _)| o.as_ref().unwrap().text.as_str()).collect();
    assert_eq!(text, ["one", "two", "one", "two"]);
}

#[test]
fn evicted_output_reports_truncation_and_retains_the_tail() {
    let stream = format!(
        "\x1b]133;A\x07$ \x1b]133;B\x07command\r\n\x1b]133;C\x07{}tail\x1b]133;D;0\x07",
        "output\r\n".repeat(30)
    );
    for (output, _) in outputs(&stream, 80, 2) {
        let output = output.unwrap();
        assert!(output.truncated_head);
        assert!(output.text.ends_with("tail"));
        assert!(!output.text.contains("command"));
    }
}

#[test]
fn alternate_screen_redraws_are_not_captured_as_output() {
    let stream = concat!(
        "\x1b]133;A\x07$ \x1b]133;B\x07tui\r\n\x1b]133;C\x07",
        "\x1b[?1049hTUI\x1b[?1049l\x1b]133;D;0\x07",
    );
    for (output, alt) in outputs(stream, 80, 100) {
        assert!(alt);
        assert!(output.is_none());
    }
}
