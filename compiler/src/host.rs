// Platform layer: where output goes, where input comes from, the clock and interrupts.
// Native builds use the terminal. The WebAssembly build (see wasm.rs) calls functions
// that the page's Web Worker provides (static/arython-worker.js).

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub enum Input {
    Line(String),
    Eof,
    Interrupt,
    /// The page can't ask for input (no shared memory)
    Unsupported,
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::Input;
    use std::io::{IsTerminal, Write};

    pub fn stdout_is_terminal() -> bool {
        std::io::stdout().is_terminal()
    }

    pub fn write_out(s: &str) {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(s.as_bytes());
        let _ = stdout.flush();
    }

    pub fn read_line(prompt: &str) -> Input {
        write_out(prompt);
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => Input::Eof,
            Ok(_) => {
                while line.ends_with('\n') || line.ends_with('\r') {
                    line.pop();
                }
                Input::Line(line)
            }
        }
    }

    pub fn now_secs() -> f64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
    }

    /// Returns false if interrupted
    pub fn sleep(secs: f64) -> bool {
        std::thread::sleep(std::time::Duration::from_secs_f64(secs));
        true
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::Input;

    pub const EMIT_STDOUT: u32 = 0;
    pub const EMIT_RESULT: u32 = 1;
    pub const EMIT_ERROR: u32 = 2;

    const INPUT_EOF: i32 = -1;
    const INPUT_INTERRUPT: i32 = -2;
    const INPUT_UNSUPPORTED: i32 = -3;
    const INPUT_BYTES: usize = 64 * 1024;

    mod ffi {
        #[link(wasm_import_module = "host")]
        extern "C" {
            /// kind: EMIT_*; line is only used for errors (0 = none)
            pub fn emit(kind: u32, ptr: *const u8, len: usize, line: u32);
            /// Writes the reply into out (at most cap bytes); returns its length or INPUT_*
            pub fn input(prompt: *const u8, prompt_len: usize, out: *mut u8, cap: usize) -> i32;
            pub fn now_ms() -> f64;
            /// Returns 0 if interrupted
            pub fn sleep_ms(ms: f64) -> u32;
            pub fn interrupted() -> u32;
        }
    }

    pub fn emit_text(kind: u32, s: &str, line: u32) {
        unsafe { ffi::emit(kind, s.as_ptr(), s.len(), line) }
    }

    pub fn stdout_is_terminal() -> bool {
        false
    }

    pub fn write_out(s: &str) {
        emit_text(EMIT_STDOUT, s, 0);
    }

    pub fn read_line(prompt: &str) -> Input {
        let mut buf = vec![0u8; INPUT_BYTES];
        match unsafe { ffi::input(prompt.as_ptr(), prompt.len(), buf.as_mut_ptr(), buf.len()) } {
            INPUT_EOF => Input::Eof,
            INPUT_INTERRUPT => Input::Interrupt,
            INPUT_UNSUPPORTED => Input::Unsupported,
            n => {
                buf.truncate(n.max(0) as usize);
                Input::Line(String::from_utf8_lossy(&buf).into_owned())
            }
        }
    }

    pub fn now_secs() -> f64 {
        unsafe { ffi::now_ms() / 1000.0 }
    }

    pub fn sleep(secs: f64) -> bool {
        unsafe { ffi::sleep_ms(secs * 1000.0) != 0 }
    }

    pub fn interrupted() -> bool {
        unsafe { ffi::interrupted() != 0 }
    }
}

pub use imp::*;
