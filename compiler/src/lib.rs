// أرايثون: a standalone Arabic programming language.
// The interpreter as a library, used by the command-line program (main.rs)
// and by the WebAssembly build for the browser (wasm.rs).

mod alloc;
mod ast;
mod builtins;
mod host;
pub mod interp;
mod lexer;
mod parser;
mod resolve;
mod value;
#[cfg(target_arch = "wasm32")]
mod wasm;

use interp::Interp;
use value::Value;

#[global_allocator]
static ALLOC: alloc::Allocator = alloc::Allocator::new();

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// An error that stopped the program
pub struct Failure {
    /// Exception class name, e.g. "ZeroDivisionError"
    pub kind: String,
    pub msg: String,
    /// 0 if unknown
    pub line: u32,
}

impl Failure {
    fn from_exc(it: &mut Interp, e: &interp::Exc) -> Failure {
        let kind = match &e.value {
            Value::Instance(i) => i.class.name.to_string(),
            _ => "Error".into(),
        };
        let msg = it.to_str(&e.value).unwrap_or_default();
        Failure { kind, msg, line: e.line }
    }

    fn from_syntax(e: lexer::SyntaxError) -> Failure {
        Failure { kind: e.kind.into(), msg: e.msg, line: e.line }
    }
}

/// Parses, resolves and runs one piece of code in `it`, then flushes its output.
/// With `echo_last`, a trailing expression is evaluated separately and the repr of
/// its value returned (None for None), like Python's REPL.
pub fn run_source(it: &mut Interp, src: &str, echo_last: bool) -> Result<Option<String>, Failure> {
    let r = run_inner(it, src, echo_last);
    it.flush();
    r
}

fn run_inner(it: &mut Interp, src: &str, echo_last: bool) -> Result<Option<String>, Failure> {
    let src = src.replace("\r\n", "\n");
    let mut body = parser::parse_program(&src).map_err(Failure::from_syntax)?;
    resolve::resolve_module(&mut body, &mut it.names).map_err(Failure::from_syntax)?;
    it.sync_globals();

    let last = match body.last().map(|s| &s.kind) {
        Some(ast::StmtKind::Expr(_)) if echo_last => body.pop(),
        _ => None,
    };
    if let Err(e) = it.run(&body) {
        return Err(Failure::from_exc(it, &e));
    }
    let Some(stmt) = last else { return Ok(None) };
    let ast::StmtKind::Expr(e) = &stmt.kind else { unreachable!() };
    let mut frame = interp::Frame::module();
    match it.eval(&mut frame, e).and_then(|v| match v {
        Value::None => Ok(None),
        v => it.repr(&v).map(Some),
    }) {
        Ok(r) => Ok(r),
        Err(mut err) => {
            err.line = stmt.line;
            Err(Failure::from_exc(it, &err))
        }
    }
}
