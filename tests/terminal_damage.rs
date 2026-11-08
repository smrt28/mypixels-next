//! Retained rendering must distinguish real writes from an idle cursor.
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermDamage};
use alacritty_terminal::vte::ansi::Processor;

#[test]
fn retained_damage_keeps_writes_but_does_not_redirty_an_idle_cursor() {
    let mut term = Term::new(Config::default(), &TermSize::new(40, 10), VoidListener);
    assert!(matches!(term.damage_content(), TermDamage::Full));
    term.reset_damage();
    let TermDamage::Partial(lines) = term.damage_content() else {
        panic!("idle full damage")
    };
    assert_eq!(lines.count(), 0);

    let mut parser: Processor = Processor::default();
    // Write then restore the cursor: the final cursor state alone cannot tell
    // whether its cell changed. This must still reach the retained texture.
    parser.advance(&mut term, b"X\x1b[D");
    let TermDamage::Partial(lines) = term.damage_content() else {
        panic!("write full damage")
    };
    assert!(
        lines
            .into_iter()
            .any(|line| line.line == 0 && line.left == 0)
    );
    term.reset_damage();
    let TermDamage::Partial(lines) = term.damage_content() else {
        panic!("idle full damage")
    };
    assert_eq!(lines.count(), 0);

    // Preserve the upstream API's promise to repaint the cursor every query.
    let TermDamage::Partial(lines) = term.damage() else {
        panic!("cursor full damage")
    };
    assert_eq!(lines.count(), 1);
}
