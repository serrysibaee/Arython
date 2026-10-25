// Entry points for the WebAssembly build, called by static/arython-worker.js.
// The page passes code by writing UTF-8 into a buffer from `alloc`; output, results
// and errors come back through the `host` imports (see host.rs).

use crate::host::{emit_text, EMIT_ERROR, EMIT_RESULT};
use crate::interp::Interp;
use std::cell::RefCell;

thread_local! {
    /// The notebook's session: variables persist between cells
    static KERNEL: RefCell<Option<Interp>> = const { RefCell::new(None) };
}

/// A buffer of `len` bytes for the page to write code into; the run_* call frees it
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
}

unsafe fn take_code(ptr: *mut u8, len: usize) -> String {
    let bytes = Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len));
    String::from_utf8_lossy(&bytes).into_owned()
}

fn run(it: &mut Interp, code: &str, echo_last: bool) {
    match crate::run_source(it, code, echo_last) {
        Ok(Some(r)) => emit_text(EMIT_RESULT, &r, 0),
        Ok(None) => {}
        Err(f) => {
            let text = if f.msg.is_empty() { f.kind } else { format!("{}: {}", f.kind, f.msg) };
            emit_text(EMIT_ERROR, &text, f.line);
        }
    }
}

/// Notebook: runs one cell in the shared session, showing a trailing expression's value
#[no_mangle]
pub unsafe extern "C" fn run_cell(ptr: *mut u8, len: usize) {
    let code = take_code(ptr, len);
    KERNEL.with(|k| run(k.borrow_mut().get_or_insert_with(Interp::new), &code, true));
}

/// Editor: runs a whole program in a fresh session
#[no_mangle]
pub unsafe extern "C" fn run_script(ptr: *mut u8, len: usize) {
    let code = take_code(ptr, len);
    run(&mut Interp::new(), &code, false);
}
