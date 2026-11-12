use buzz_terminal::damage::Encoder;
use buzz_terminal::fences::Fences;
use buzz_terminal::{SharedTerminal, Size, Terminal};

#[test]
fn space_over_blank_cell_publishes_cursor_only_frame() {
    let (terminal, _actions) = Terminal::new(
        Size {
            columns: 8,
            screen_lines: 2,
            scrollback: 10,
        },
        Fences::ALL,
    );
    let terminal = SharedTerminal::new(terminal);
    let mut encoder = Encoder::new();

    let initial = terminal.render(&mut encoder);
    assert!(!initial.is_empty());
    assert_eq!(initial.cursor.column, 0);

    terminal.feed_fully(b" ");
    let after_space = terminal.render(&mut encoder);

    assert!(
        after_space.rows.is_empty(),
        "a blank cell overwritten with a space must be row-deduplicated"
    );
    assert_eq!(after_space.cursor.column, 1);
    assert!(after_space.cursor_changed);
    assert!(
        !after_space.is_empty(),
        "cursor movement must make the frame publishable"
    );

    let idle = terminal.render(&mut encoder);
    assert!(
        idle.is_empty(),
        "an unchanged cursor must not create traffic"
    );
}
