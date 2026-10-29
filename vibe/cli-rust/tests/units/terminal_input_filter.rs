//! Terminal color reports never become composer key events.

use std::time::{Duration, Instant};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use vibe_rs::terminal_input_filter::TerminalInputFilter;

fn key(character: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
}

fn modified(character: char, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(character), modifiers))
}

fn release(character: char, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char(character),
        modifiers,
        KeyEventKind::Release,
    ))
}

fn esc() -> Event {
    Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
}

fn esc_release() -> Event {
    Event::Key(KeyEvent::new_with_kind(
        KeyCode::Esc,
        KeyModifiers::NONE,
        KeyEventKind::Release,
    ))
}

fn mouse_move() -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: 1,
        row: 1,
        modifiers: KeyModifiers::NONE,
    })
}

fn report_events(terminator: Event) -> Vec<Event> {
    let mut events = vec![modified(']', KeyModifiers::ALT)];
    events.extend("11;rgb:1e1e/1e1e/2e2e".chars().map(key));
    events.push(terminator);
    events
}

/// An SGR mouse report as Crossterm exposes it after a read split on its ESC.
fn split_mouse_report(terminator: Event) -> Vec<Event> {
    let mut events = vec![esc()];
    events.extend("[<35;128;2".chars().map(key));
    events.push(terminator);
    events
}

#[test]
fn drops_bell_terminated_color_report() {
    let mut filter = TerminalInputFilter::default();
    let output: Vec<_> = report_events(modified('g', KeyModifiers::CONTROL))
        .into_iter()
        .flat_map(|event| filter.push(event))
        .collect();

    assert!(output.is_empty());
}

#[test]
fn drops_string_terminated_color_report_and_keeps_following_key() {
    let mut filter = TerminalInputFilter::default();
    let mut output: Vec<_> = report_events(modified('\\', KeyModifiers::ALT))
        .into_iter()
        .flat_map(|event| filter.push(event))
        .collect();
    output.extend(filter.push(key('x')));

    assert_eq!(output, [key('x')]);
}

#[test]
fn drops_the_report_start_when_the_sequence_is_not_a_color_report() {
    let mut filter = TerminalInputFilter::default();
    let start = modified(']', KeyModifiers::ALT);

    assert!(filter.push(start).is_empty());
    assert_eq!(filter.push(key('x')), [key('x')]);
}

#[test]
fn drops_a_non_ascii_color_candidate_and_keeps_the_diverging_key() {
    let mut filter = TerminalInputFilter::default();
    let mut events = vec![modified(']', KeyModifiers::ALT)];
    events.extend("1;rgb".chars().map(key));
    let diverging = key('é');
    events.push(diverging.clone());

    let output: Vec<_> = events
        .into_iter()
        .flat_map(|event| filter.push(event))
        .collect();

    assert_eq!(output, [diverging]);
}

#[test]
fn drops_color_report_interleaved_with_releases_and_mouse() {
    let mut filter = TerminalInputFilter::default();
    let mut events = vec![
        modified(']', KeyModifiers::ALT),
        release(']', KeyModifiers::ALT),
        mouse_move(),
        Event::Resize(80, 24),
        Event::FocusLost,
        Event::FocusGained,
    ];
    events.extend(
        "11;rgb:1e1e/1e1e/2e2e"
            .chars()
            .flat_map(|character| [key(character), release(character, KeyModifiers::NONE)]),
    );
    events.push(modified('g', KeyModifiers::CONTROL));

    let output: Vec<_> = events
        .into_iter()
        .flat_map(|event| filter.push(event))
        .collect();

    assert_eq!(
        output,
        [
            mouse_move(),
            Event::Resize(80, 24),
            Event::FocusLost,
            Event::FocusGained,
        ]
    );
}

#[test]
fn holds_the_match_across_noise_then_delivers_the_real_key() {
    let mut filter = TerminalInputFilter::default();
    let start = modified(']', KeyModifiers::ALT);

    assert!(filter.push(start).is_empty());
    assert!(filter.push(release(']', KeyModifiers::ALT)).is_empty());
    assert_eq!(filter.push(mouse_move()), [mouse_move()]);
    assert_eq!(filter.push(key('x')), [key('x')]);
}

#[test]
fn drops_an_incomplete_candidate_after_the_bound() {
    let mut filter = TerminalInputFilter::default();
    let start = modified(']', KeyModifiers::ALT);

    assert!(filter.push(start).is_empty());
    assert!(filter
        .flush_expired(Instant::now() + Duration::from_secs(1))
        .is_empty());
    // The filter is clean again: the next key passes straight through.
    assert_eq!(filter.push(key('x')), [key('x')]);
}

#[test]
fn delivers_a_user_key_that_interrupts_a_partial_report() {
    let mut filter = TerminalInputFilter::default();
    let down = Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

    let mut events = vec![modified(']', KeyModifiers::ALT)];
    events.extend("11;rgb:1e1e/1e1e/1e1e".chars().map(key));

    let output: Vec<_> = events
        .into_iter()
        .flat_map(|event| filter.push(event))
        .collect();
    assert!(output.is_empty());

    // The user presses Down mid-report: the report bytes are dropped, the
    // Down is delivered alone instead of joining a replayed junk batch.
    assert_eq!(filter.push(down.clone()), [down]);
    // And the stream is clean for the next key.
    assert_eq!(filter.push(key('x')), [key('x')]);
}

#[test]
fn drops_a_split_sgr_mouse_report() {
    let mut filter = TerminalInputFilter::default();
    let output: Vec<_> = split_mouse_report(modified('M', KeyModifiers::SHIFT))
        .into_iter()
        .flat_map(|event| filter.push(event))
        .collect();

    assert!(output.is_empty());
    // The stream is clean for the next key.
    assert_eq!(filter.push(key('x')), [key('x')]);
}

#[test]
fn drops_a_split_sgr_mouse_release_report_interleaved_with_releases() {
    let mut filter = TerminalInputFilter::default();
    let mut events = vec![esc(), esc_release()];
    events.extend(
        "[<64;20;5"
            .chars()
            .flat_map(|character| [key(character), release(character, KeyModifiers::NONE)]),
    );
    events.push(key('m'));

    let output: Vec<_> = events
        .into_iter()
        .flat_map(|event| filter.push(event))
        .collect();

    assert!(output.is_empty());
}

#[test]
fn drops_a_split_report_interleaved_with_noise() {
    let mut filter = TerminalInputFilter::default();
    let mut events = vec![esc(), mouse_move(), Event::Resize(80, 24)];
    events.extend("[<35;1".chars().map(key));
    events.push(Event::FocusLost);
    events.extend("28;2".chars().map(key));
    events.push(modified('M', KeyModifiers::SHIFT));

    let output: Vec<_> = events
        .into_iter()
        .flat_map(|event| filter.push(event))
        .collect();

    assert_eq!(
        output,
        [mouse_move(), Event::Resize(80, 24), Event::FocusLost]
    );
}

#[test]
fn delivers_a_lone_escape_when_nothing_follows() {
    let mut filter = TerminalInputFilter::default();

    assert!(filter.push(esc()).is_empty());
    assert_eq!(
        filter.flush_expired(Instant::now() + Duration::from_millis(51)),
        [esc()]
    );
}

#[test]
fn delivers_an_escape_before_a_diverging_key() {
    let mut filter = TerminalInputFilter::default();

    assert!(filter.push(esc()).is_empty());
    assert_eq!(filter.push(key('a')), [esc(), key('a')]);
}

#[test]
fn delivers_an_escape_before_an_interrupting_key() {
    let mut filter = TerminalInputFilter::default();
    let down = Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

    assert!(filter.push(esc()).is_empty());
    assert_eq!(filter.push(down.clone()), [esc(), down]);
}

#[test]
fn keeps_keys_typed_after_an_escape_and_a_bracket() {
    let mut filter = TerminalInputFilter::default();

    assert!(filter.push(esc()).is_empty());
    assert!(filter.push(key('[')).is_empty());
    // The residue diverges at 'h': everything held is the user's own.
    assert_eq!(filter.push(key('h')), [esc(), key('['), key('h')]);
}

#[test]
fn delivers_a_partial_residue_when_the_report_stalls() {
    let mut filter = TerminalInputFilter::default();

    assert!(filter.push(esc()).is_empty());
    assert!(filter.push(key('[')).is_empty());
    assert!(filter.push(key('<')).is_empty());
    assert_eq!(
        filter.flush_expired(Instant::now() + Duration::from_millis(51)),
        [esc(), key('['), key('<')]
    );
}

#[test]
fn delivers_a_residue_typed_without_a_preceding_escape() {
    let mut filter = TerminalInputFilter::default();
    let typed: Vec<Event> = "[<35;128;2"
        .chars()
        .map(key)
        .chain(std::iter::once(modified('M', KeyModifiers::SHIFT)))
        .collect();

    let output: Vec<_> = typed
        .clone()
        .into_iter()
        .flat_map(|event| filter.push(event))
        .collect();

    assert_eq!(output, typed);
}

#[test]
fn delivers_a_modified_escape_immediately() {
    let mut filter = TerminalInputFilter::default();
    let alt_esc = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::ALT));

    assert_eq!(filter.push(alt_esc.clone()), [alt_esc]);
    assert_eq!(filter.push(esc_release()), [esc_release()]);
}
