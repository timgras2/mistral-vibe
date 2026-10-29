//! Filters terminal reports that Crossterm otherwise exposes as key presses.

use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

const MAX_REPORT_EVENTS: usize = 64;
const PARTIAL_REPORT_TIMEOUT: Duration = Duration::from_millis(500);
/// A parked Escape flushes within one poll interval: a lone Escape key, never a report byte.
const SPLIT_MOUSE_TIMEOUT: Duration = Duration::from_millis(50);

/// What a held candidate's buffered bytes are.
#[derive(Default, PartialEq)]
enum Candidate {
    #[default]
    Report,
    /// An Escape that may be the first byte of a split SGR mouse report.
    Mouse,
}

#[derive(Default)]
pub struct TerminalInputFilter {
    pending: Vec<Event>,
    payload: String,
    started: Option<Instant>,
    kind: Candidate,
    program_status_reply: bool,
}

impl TerminalInputFilter {
    pub fn push(&mut self, event: Event) -> Vec<Event> {
        if self.pending.is_empty() {
            if is_osc_start(&event) {
                self.started = Some(Instant::now());
                self.kind = Candidate::Report;
                self.pending.push(event);
                self.payload.push(']');
                return Vec::new();
            }
            if is_split_mouse_start(&event) {
                self.started = Some(Instant::now());
                self.kind = Candidate::Mouse;
                self.pending.push(event);
                return Vec::new();
            }
            return vec![event];
        }

        if !is_matching_key(&event) {
            return match event {
                Event::Key(_) => Vec::new(),
                event => vec![event],
            };
        }

        if self.kind == Candidate::Report && is_osc_terminator(&event) {
            // OSC 7501 revision 0.3: the reply starts with ?, future fields are ignored.
            self.program_status_reply |= self.payload.starts_with("]7501;?");
            // Report or not, the buffered bytes are terminal noise, never input.
            self.clear();
            return Vec::new();
        }
        if self.kind == Candidate::Mouse && is_mouse_terminator(&self.payload, &event) {
            // A split SGR mouse report: the Escape and its residue are terminal noise.
            self.clear();
            return Vec::new();
        }

        let Some(character) = plain_character(&event) else {
            // A real key interrupts the candidate: report noise is dropped, the key is not.
            return self.interrupt(event);
        };
        if self.kind == Candidate::Report
            && self.payload.starts_with("]7501;?")
            && self.pending.len() + 1 >= MAX_REPORT_EVENTS
        {
            // Ignore future reply fields without growing the buffer or leaking them as keys.
            return Vec::new();
        }
        self.payload.push(character);
        let partial = match self.kind {
            Candidate::Report => is_partial_report(&self.payload),
            Candidate::Mouse => is_partial_mouse_residue(&self.payload),
        };
        let diverged = self.pending.len() + 1 >= MAX_REPORT_EVENTS || !partial;
        self.pending.push(event);
        if diverged {
            // The payload diverged: the buffered bytes may be the user's own.
            return self.diverge();
        }
        if self.kind == Candidate::Mouse {
            // A progressing residue outlives a stalled read.
            self.started = Some(Instant::now());
        }
        Vec::new()
    }

    pub fn flush_expired(&mut self, now: Instant) -> Vec<Event> {
        let Some(started) = self.started else {
            return Vec::new();
        };
        let timeout = match self.kind {
            Candidate::Report => PARTIAL_REPORT_TIMEOUT,
            Candidate::Mouse => SPLIT_MOUSE_TIMEOUT,
        };
        if now.duration_since(started) < timeout {
            return Vec::new();
        }
        if self.kind == Candidate::Mouse {
            // Nothing completed the report: the Escape was the user's key after all.
            return self.take_pending();
        }
        // An unfinished report is terminal noise; it never becomes input.
        self.clear();
        Vec::new()
    }

    pub fn take_program_status_reply(&mut self) -> bool {
        std::mem::take(&mut self.program_status_reply)
    }

    /// A real key interrupts a candidate: report noise is dropped, held keys are not.
    fn interrupt(&mut self, event: Event) -> Vec<Event> {
        if self.kind == Candidate::Mouse {
            let mut out = self.take_pending();
            out.push(event);
            return out;
        }
        self.clear();
        vec![event]
    }

    /// The payload diverged from its report: deliver what may be the user's own keys.
    fn diverge(&mut self) -> Vec<Event> {
        if self.kind == Candidate::Mouse {
            return self.take_pending();
        }
        let event = self.pending.pop().expect("pushed above");
        self.clear();
        vec![event]
    }

    fn take_pending(&mut self) -> Vec<Event> {
        self.payload.clear();
        self.started = None;
        std::mem::take(&mut self.pending)
    }

    fn clear(&mut self) {
        self.take_pending();
    }
}

fn is_matching_key(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(KeyEvent {
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        })
    )
}

fn is_osc_start(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(KeyEvent {
            code: KeyCode::Char(']'),
            modifiers,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        }) if *modifiers == KeyModifiers::ALT
    )
}

fn is_osc_terminator(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(KeyEvent {
            code: KeyCode::Char('g'),
            modifiers,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        }) if *modifiers == KeyModifiers::CONTROL
    ) || matches!(
        event,
        Event::Key(KeyEvent {
            code: KeyCode::Char('\\'),
            modifiers,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        }) if *modifiers == KeyModifiers::ALT
    )
}

/// A bare Escape press: the user's key, or the first byte of a split mouse report.
fn is_split_mouse_start(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(KeyEvent {
            code: KeyCode::Esc,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) if modifiers.is_empty()
    )
}

/// The final `M`/`m` of a split SGR mouse report with a complete body.
fn is_mouse_terminator(payload: &str, event: &Event) -> bool {
    is_complete_mouse_residue(payload) && matches!(plain_character(event), Some('M' | 'm'))
}

fn plain_character(event: &Event) -> Option<char> {
    let Event::Key(key) = event else {
        return None;
    };
    if key.kind == KeyEventKind::Release || !(key.modifiers - KeyModifiers::SHIFT).is_empty() {
        return None;
    }
    match key.code {
        KeyCode::Char(character) => Some(character),
        _ => None,
    }
}

/// A partial residue of a split SGR mouse report: `[<Cb;Cx;Cy` without its final `M`/`m`.
fn is_partial_mouse_residue(payload: &str) -> bool {
    let Some(rest) = payload.strip_prefix("[<") else {
        return payload == "[";
    };
    if rest.is_empty() {
        return true;
    }
    let mut numbers = rest.split(';');
    let first = numbers.next().expect("a non-empty rest splits once");
    if first.is_empty() || !first.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let mut count = 1;
    for number in numbers {
        count += 1;
        if count > 3 || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
    }
    true
}

/// A complete residue body: `[<Cb;Cx;Cy` with its three numbers.
fn is_complete_mouse_residue(payload: &str) -> bool {
    let Some(rest) = payload.strip_prefix("[<") else {
        return false;
    };
    let numbers: Vec<&str> = rest.split(';').collect();
    numbers.len() == 3
        && numbers
            .iter()
            .all(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_partial_report(payload: &str) -> bool {
    let reply = "]7501;?";
    if reply.starts_with(payload) || payload.starts_with(reply) {
        return true;
    }
    let Some(payload) = payload.strip_prefix(']') else {
        return false;
    };
    let digit_count = payload.chars().take_while(char::is_ascii_digit).count();
    if digit_count == 0 || digit_count > 3 {
        return payload.is_empty();
    }
    if digit_count == payload.len() {
        return true;
    }
    let expected = b";rgb:";
    let remainder = &payload.as_bytes()[digit_count..];
    let shared = remainder.len().min(expected.len());
    if remainder[..shared] != expected[..shared] {
        return false;
    }
    if remainder.len() <= expected.len() {
        return true;
    }
    let color = &remainder[expected.len()..];
    !color.is_empty()
        && color
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() || *byte == b'/')
}
