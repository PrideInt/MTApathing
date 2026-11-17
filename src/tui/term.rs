//! Terminal plumbing with no dependencies: raw mode, alternate screen, size,
//! and key input on Unix (macOS/Linux) and Windows 10+ consoles.
//!
//! Both platforms are driven the same way. The console is put into a mode
//! where it emits VT escape sequences for keys and interprets them on output,
//! so one key parser and one renderer serve every OS.

use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// Ctrl + letter (lowercase), e.g. Ctrl('c').
    Ctrl(char),
}

/// Puts the terminal into raw mode on creation and restores it on drop
/// (including during a panic unwind).
pub struct Terminal {
    #[cfg(unix)]
    saved_stty: Option<String>,
    #[cfg(windows)]
    saved_modes: Option<(u32, u32)>,
}

const ENTER_SEQ: &str = "\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[2J";
const LEAVE_SEQ: &str = "\x1b[0m\x1b[2J\x1b[?7h\x1b[?25h\x1b[?1049l";

impl Terminal {
    pub fn enter() -> io::Result<Terminal> {
        let t = Terminal::raw_mode()?;
        let mut out = io::stdout();
        out.write_all(ENTER_SEQ.as_bytes())?;
        out.flush()?;
        Ok(t)
    }

    #[cfg(unix)]
    fn raw_mode() -> io::Result<Terminal> {
        use std::process::{Command, Stdio};
        let saved = Command::new("stty")
            .arg("-g")
            .stdin(Stdio::inherit())
            .output()?;
        let saved = String::from_utf8_lossy(&saved.stdout).trim().to_string();
        let ok = Command::new("stty")
            .args(["raw", "-echo"])
            .stdin(Stdio::inherit())
            .status()?
            .success();
        if !ok {
            return Err(io::Error::other("stty raw failed"));
        }
        Ok(Terminal {
            saved_stty: Some(saved),
        })
    }

    #[cfg(windows)]
    fn raw_mode() -> io::Result<Terminal> {
        let modes =
            win::enable_raw_vt().map_err(|e| io::Error::other(format!("console mode: {e}")))?;
        Ok(Terminal {
            saved_modes: Some(modes),
        })
    }

    #[cfg(not(any(unix, windows)))]
    fn raw_mode() -> io::Result<Terminal> {
        Err(io::Error::other("unsupported platform"))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = out.write_all(LEAVE_SEQ.as_bytes());
        let _ = out.flush();
        #[cfg(unix)]
        if let Some(s) = self.saved_stty.take() {
            let _ = std::process::Command::new("stty")
                .arg(s)
                .stdin(std::process::Stdio::inherit())
                .status();
        }
        #[cfg(windows)]
        if let Some(m) = self.saved_modes.take() {
            win::restore(m);
        }
    }
}

/// Terminal size as (columns, rows).
pub fn size() -> (u16, u16) {
    platform_size()
        .filter(|(w, h)| *w > 0 && *h > 0)
        .or_else(env_size)
        .unwrap_or((80, 24))
}

fn env_size() -> Option<(u16, u16)> {
    let c = std::env::var("COLUMNS").ok()?.parse().ok()?;
    let l = std::env::var("LINES").ok()?.parse().ok()?;
    Some((c, l))
}

#[cfg(unix)]
fn platform_size() -> Option<(u16, u16)> {
    #[repr(C)]
    struct WinSize {
        rows: u16,
        cols: u16,
        xpix: u16,
        ypix: u16,
    }
    extern "C" {
        fn ioctl(fd: i32, request: std::ffi::c_ulong, ...) -> i32;
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const TIOCGWINSZ: std::ffi::c_ulong = 0x5413;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const TIOCGWINSZ: std::ffi::c_ulong = 0x4008_7468; // macOS / BSD
    let mut ws = WinSize {
        rows: 0,
        cols: 0,
        xpix: 0,
        ypix: 0,
    };
    for fd in [1, 0, 2] {
        // SAFETY: TIOCGWINSZ writes a `struct winsize` into the pointer we pass.
        let r = unsafe { ioctl(fd, TIOCGWINSZ, &mut ws as *mut WinSize) };
        if r == 0 && ws.cols > 0 {
            return Some((ws.cols, ws.rows));
        }
    }
    None
}

#[cfg(windows)]
fn platform_size() -> Option<(u16, u16)> {
    win::size()
}

#[cfg(not(any(unix, windows)))]
fn platform_size() -> Option<(u16, u16)> {
    None
}

/// Read stdin on a background thread; chunks of raw bytes arrive on the channel.
pub fn spawn_input() -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut buf = [0u8; 256];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
    rx
}

/// Parse as many complete keys as possible from `buf`, leaving any incomplete
/// escape or UTF-8 sequence in place. If `flush_esc` is set, a lone ESC is
/// taken to be the Esc key.
pub fn parse_keys(buf: &mut Vec<u8>, flush_esc: bool) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        let b = buf[i];
        let rest = &buf[i..];
        let (key, used) = match b {
            0x1b => match parse_escape(rest) {
                Some(r) => r,
                None if flush_esc => (Some(Key::Esc), 1),
                None => break,
            },
            b'\r' | b'\n' => (Some(Key::Enter), 1),
            b'\t' => (Some(Key::Tab), 1),
            0x7f | 0x08 => (Some(Key::Backspace), 1),
            0x01..=0x1a => (Some(Key::Ctrl((b - 1 + b'a') as char)), 1),
            0x00 | 0x1c..=0x1f => (None, 1),
            _ => {
                let len = match b {
                    0xc0..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf7 => 4,
                    0x80..=0xbf => {
                        i += 1; // stray continuation byte
                        continue;
                    }
                    _ => 1,
                };
                if rest.len() < len {
                    break;
                }
                match std::str::from_utf8(&rest[..len])
                    .ok()
                    .and_then(|s| s.chars().next())
                {
                    Some(c) => (Some(Key::Char(c)), len),
                    None => (None, 1),
                }
            }
        };
        if let Some(k) = key {
            keys.push(k);
        }
        i += used;
    }
    buf.drain(..i);
    keys
}

/// Returns None if the sequence is incomplete.
fn parse_escape(s: &[u8]) -> Option<(Option<Key>, usize)> {
    if s.len() < 2 {
        return None;
    }
    match s[1] {
        b'[' => {
            // CSI: params (0x30–0x3F), intermediates (0x20–0x2F), final (0x40–0x7E)
            let mut j = 2;
            while j < s.len() && (0x20..=0x3f).contains(&s[j]) {
                j += 1;
            }
            if j >= s.len() {
                return None;
            }
            let params = std::str::from_utf8(&s[2..j]).unwrap_or("");
            let first: u32 = params
                .split(';')
                .next()
                .and_then(|p| p.parse().ok())
                .unwrap_or(0);
            let key = match s[j] {
                b'A' => Some(Key::Up),
                b'B' => Some(Key::Down),
                b'C' => Some(Key::Right),
                b'D' => Some(Key::Left),
                b'H' => Some(Key::Home),
                b'F' => Some(Key::End),
                b'Z' => Some(Key::BackTab),
                b'~' => match first {
                    1 | 7 => Some(Key::Home),
                    4 | 8 => Some(Key::End),
                    3 => Some(Key::Delete),
                    5 => Some(Key::PageUp),
                    6 => Some(Key::PageDown),
                    _ => None,
                },
                _ => None,
            };
            Some((key, j + 1))
        }
        b'O' => {
            if s.len() < 3 {
                return None;
            }
            let key = match s[2] {
                b'A' => Some(Key::Up),
                b'B' => Some(Key::Down),
                b'C' => Some(Key::Right),
                b'D' => Some(Key::Left),
                b'H' => Some(Key::Home),
                b'F' => Some(Key::End),
                _ => None,
            };
            Some((key, 3))
        }
        0x1b => Some((Some(Key::Esc), 1)),
        _ => Some((Some(Key::Esc), 1)), // Alt+key: treat as Esc, then the key
    }
}

/// Windows console API, declared by hand (no `windows-sys` dependency).
///
/// Compiled on Windows; also compiled with `--cfg win_check` on other
/// platforms so `cargo check` can type-check it without a Windows toolchain.
#[cfg(any(windows, win_check))]
#[allow(clippy::upper_case_acronyms, dead_code)]
pub mod win {
    use std::ffi::c_void;

    type HANDLE = *mut c_void;
    type BOOL = i32;
    type DWORD = u32;

    const STD_INPUT_HANDLE: DWORD = -10i32 as DWORD;
    const STD_OUTPUT_HANDLE: DWORD = -11i32 as DWORD;
    const INVALID_HANDLE_VALUE: isize = -1;

    // Input modes
    const ENABLE_PROCESSED_INPUT: DWORD = 0x0001;
    const ENABLE_LINE_INPUT: DWORD = 0x0002;
    const ENABLE_ECHO_INPUT: DWORD = 0x0004;
    const ENABLE_QUICK_EDIT_MODE: DWORD = 0x0040;
    const ENABLE_EXTENDED_FLAGS: DWORD = 0x0080;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: DWORD = 0x0200;
    // Output modes
    const ENABLE_PROCESSED_OUTPUT: DWORD = 0x0001;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: DWORD = 0x0004;
    const DISABLE_NEWLINE_AUTO_RETURN: DWORD = 0x0008;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct COORD {
        x: i16,
        y: i16,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct SMALL_RECT {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct CONSOLE_SCREEN_BUFFER_INFO {
        size: COORD,
        cursor_position: COORD,
        attributes: u16,
        window: SMALL_RECT,
        maximum_window_size: COORD,
    }

    #[cfg_attr(windows, link(name = "kernel32"))]
    extern "system" {
        fn GetStdHandle(n: DWORD) -> HANDLE;
        fn GetConsoleMode(h: HANDLE, mode: *mut DWORD) -> BOOL;
        fn SetConsoleMode(h: HANDLE, mode: DWORD) -> BOOL;
        fn GetConsoleScreenBufferInfo(h: HANDLE, info: *mut CONSOLE_SCREEN_BUFFER_INFO) -> BOOL;
    }

    fn handle(which: DWORD) -> Result<HANDLE, String> {
        // SAFETY: GetStdHandle has no preconditions.
        let h = unsafe { GetStdHandle(which) };
        if h.is_null() || h as isize == INVALID_HANDLE_VALUE {
            Err("no console handle".into())
        } else {
            Ok(h)
        }
    }

    fn get_mode(h: HANDLE) -> Result<DWORD, String> {
        let mut m: DWORD = 0;
        // SAFETY: valid handle and out-pointer.
        if unsafe { GetConsoleMode(h, &mut m) } == 0 {
            Err("not a console (try Windows Terminal, PowerShell or cmd)".into())
        } else {
            Ok(m)
        }
    }

    /// Switch to raw VT mode; returns the previous (input, output) modes.
    pub fn enable_raw_vt() -> Result<(u32, u32), String> {
        let (hin, hout) = (handle(STD_INPUT_HANDLE)?, handle(STD_OUTPUT_HANDLE)?);
        let (old_in, old_out) = (get_mode(hin)?, get_mode(hout)?);
        let new_in = (old_in
            & !(ENABLE_LINE_INPUT
                | ENABLE_ECHO_INPUT
                | ENABLE_PROCESSED_INPUT
                | ENABLE_QUICK_EDIT_MODE))
            | ENABLE_VIRTUAL_TERMINAL_INPUT
            | ENABLE_EXTENDED_FLAGS;
        let new_out = old_out
            | ENABLE_PROCESSED_OUTPUT
            | ENABLE_VIRTUAL_TERMINAL_PROCESSING
            | DISABLE_NEWLINE_AUTO_RETURN;
        // SAFETY: valid console handles.
        unsafe {
            if SetConsoleMode(hout, new_out) == 0 {
                // Older consoles may reject DISABLE_NEWLINE_AUTO_RETURN.
                if SetConsoleMode(hout, new_out & !DISABLE_NEWLINE_AUTO_RETURN) == 0 {
                    return Err(
                        "this console doesn't support VT sequences (Windows 10+ needed)".into(),
                    );
                }
            }
            if SetConsoleMode(hin, new_in) == 0 {
                SetConsoleMode(hout, old_out);
                return Err("couldn't set console input mode".into());
            }
        }
        Ok((old_in, old_out))
    }

    pub fn restore((old_in, old_out): (u32, u32)) {
        if let (Ok(hin), Ok(hout)) = (handle(STD_INPUT_HANDLE), handle(STD_OUTPUT_HANDLE)) {
            // SAFETY: valid console handles.
            unsafe {
                SetConsoleMode(hin, old_in);
                SetConsoleMode(hout, old_out);
            }
        }
    }

    pub fn size() -> Option<(u16, u16)> {
        let h = handle(STD_OUTPUT_HANDLE).ok()?;
        let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
        // SAFETY: valid handle and out-pointer.
        if unsafe { GetConsoleScreenBufferInfo(h, &mut info) } == 0 {
            return None;
        }
        let w = info.window.right - info.window.left + 1;
        let h = info.window.bottom - info.window.top + 1;
        Some((w.max(1) as u16, h.max(1) as u16))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys_and_sequences() {
        let mut b = b"ab\x1b[A\x1b[B\x1bOC\r\x7f\t\x1b[Z\x03\x1b[5~".to_vec();
        let k = parse_keys(&mut b, false);
        assert_eq!(
            k,
            vec![
                Key::Char('a'),
                Key::Char('b'),
                Key::Up,
                Key::Down,
                Key::Right,
                Key::Enter,
                Key::Backspace,
                Key::Tab,
                Key::BackTab,
                Key::Ctrl('c'),
                Key::PageUp
            ]
        );
        assert!(b.is_empty());
    }

    #[test]
    fn keeps_incomplete_sequences() {
        let mut b = b"x\x1b[".to_vec();
        assert_eq!(parse_keys(&mut b, false), vec![Key::Char('x')]);
        assert_eq!(b, b"\x1b[");
        b.push(b'D');
        assert_eq!(parse_keys(&mut b, false), vec![Key::Left]);

        let mut b = vec![0x1b];
        assert!(parse_keys(&mut b, false).is_empty());
        assert_eq!(parse_keys(&mut b, true), vec![Key::Esc]);

        let mut b = "é".as_bytes()[..1].to_vec();
        assert!(parse_keys(&mut b, false).is_empty());
        b.push("é".as_bytes()[1]);
        assert_eq!(parse_keys(&mut b, false), vec![Key::Char('é')]);
    }
}
