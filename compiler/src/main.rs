// أرايثون: a standalone Arabic programming language.
//   arython برنامج.ary     run a program
//   arython -c "كود"       run code from the command line
//   arython                interactive prompt

mod alloc;
mod ast;
mod builtins;
mod interp;
mod lexer;
mod parser;
mod resolve;
mod value;

use interp::{Exc, Interp};
use std::io::{IsTerminal, Write};
use value::Value;

#[global_allocator]
static ALLOC: alloc::Allocator = alloc::Allocator::new();

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Deep recursion runs on this much stack before RecursionError stops it
const STACK_BYTES: usize = 512 << 20;

const ERROR_NAMES: &[(&str, &str)] = &[
    ("SyntaxError", "خطأ في الصياغة"),
    ("IndentationError", "خطأ في الإزاحة"),
    ("NameError", "اسم غير معرّف"),
    ("UnboundLocalError", "متغير بلا قيمة"),
    ("TypeError", "نوع غير مناسب"),
    ("ValueError", "قيمة غير صالحة"),
    ("ZeroDivisionError", "قسمة على صفر"),
    ("IndexError", "فهرس خارج النطاق"),
    ("KeyError", "مفتاح غير موجود"),
    ("AttributeError", "خاصية غير موجودة"),
    ("ImportError", "تعذّر الاستيراد"),
    ("ModuleNotFoundError", "وحدة غير موجودة"),
    ("RecursionError", "تكرار عودي زائد"),
    ("OverflowError", "تجاوز السعة"),
    ("EOFError", "لا يوجد إدخال"),
    ("AssertionError", "فشل التحقق"),
    ("RuntimeError", "خطأ أثناء التشغيل"),
];

fn to_arabic_digits(n: u32) -> String {
    n.to_string().chars().map(|c| char::from_u32(0x0660 + c.to_digit(10).unwrap()).unwrap()).collect()
}

fn error_text(kind: &str, msg: &str, line: u32) -> String {
    let label = ERROR_NAMES.iter().find(|(k, _)| *k == kind).map_or(kind.to_string(), |(_, ar)| format!("{ar} ({kind})"));
    let mut s = format!("⚠ {label}");
    if !msg.is_empty() {
        s.push_str(": ");
        s.push_str(msg);
    }
    if line > 0 {
        s.push_str(&format!("\n  في السطر {}", to_arabic_digits(line)));
    }
    s
}

fn report(it: &mut Interp, e: &Exc) {
    it.flush();
    let kind = match &e.value {
        Value::Instance(i) => i.class.name.to_string(),
        _ => "Error".into(),
    };
    let msg = it.to_str(&e.value).unwrap_or_default();
    eprintln!("{}", error_text(&kind, &msg, e.line));
}

fn report_syntax(e: &lexer::SyntaxError) {
    eprintln!("{}", error_text(e.kind, &e.msg, e.line));
}

/// Parses, resolves and runs one piece of code; returns false on error
fn run_source(it: &mut Interp, src: &str, echo_last: bool) -> bool {
    let src = src.replace("\r\n", "\n");
    let mut body = match parser::parse_program(&src) {
        Ok(b) => b,
        Err(e) => {
            report_syntax(&e);
            return false;
        }
    };
    if let Err(e) = resolve::resolve_module(&mut body, &mut it.names) {
        report_syntax(&e);
        return false;
    }
    it.sync_globals();

    // REPL: show the value of a trailing expression, like Python
    let last = if echo_last {
        match body.last().map(|s| &s.kind) {
            Some(ast::StmtKind::Expr(_)) => body.pop(),
            _ => None,
        }
    } else {
        None
    };
    let mut ok = match it.run(&body) {
        Ok(()) => true,
        Err(e) => {
            report(it, &e);
            false
        }
    };
    if let (true, Some(stmt)) = (ok, last) {
        if let ast::StmtKind::Expr(e) = &stmt.kind {
            let mut frame = interp::Frame::module();
            match it.eval(&mut frame, e).and_then(|v| match v {
                Value::None => Ok(None),
                v => it.repr(&v).map(Some),
            }) {
                Ok(Some(r)) => it.write(&format!("{r}\n")),
                Ok(None) => {}
                Err(mut err) => {
                    err.line = stmt.line;
                    report(it, &err);
                    ok = false;
                }
            }
        }
    }
    it.flush();
    ok
}

fn repl() {
    let mut it = Interp::new();
    println!("أرايثون {VERSION} — اكتب كودا بالعربية. للخروج: Ctrl+Z ثم Enter (أو Ctrl+D).");
    // Read one line at a time without holding the stdin lock, so input() works in the REPL
    let next_line = || {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_string()),
        }
    };
    loop {
        print!(">>> ");
        let _ = std::io::stdout().flush();
        let Some(first) = next_line() else {
            println!();
            return;
        };
        if first.trim().is_empty() {
            continue;
        }
        let mut src = first.clone();
        // Blocks (lines ending in ':') and open brackets continue until a blank line
        let needs_more = |s: &str| {
            let depth: i32 = s.chars().map(|c| match c { '(' | '[' | '{' => 1, ')' | ']' | '}' => -1, _ => 0 }).sum();
            depth > 0 || s.trim_end().ends_with(':') || s.trim_end().ends_with('\\')
        };
        if needs_more(&src) {
            loop {
                print!("... ");
                let _ = std::io::stdout().flush();
                match next_line() {
                    Some(l) if !l.trim().is_empty() => {
                        src.push('\n');
                        src.push_str(&l);
                    }
                    _ => break,
                }
            }
        }
        src.push('\n');
        run_source(&mut it, &src, true);
    }
}

fn usage() {
    println!("أرايثون {VERSION} — لغة برمجة عربية

الاستخدام:
  arython <ملف>          تشغيل برنامج
  arython -c \"<كود>\"     تشغيل كود مباشرة
  arython                الوضع التفاعلي
  arython --version      رقم الإصدار");
}

fn main_inner() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {
            if std::io::stdin().is_terminal() {
                repl();
                0
            } else {
                // Program piped on stdin
                let mut src = String::new();
                let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut src);
                let mut it = Interp::new();
                if run_source(&mut it, &src, false) { 0 } else { 1 }
            }
        }
        Some("-h" | "--help") => {
            usage();
            0
        }
        Some("-V" | "--version") => {
            println!("arython {VERSION}");
            0
        }
        Some("-c") => {
            let Some(code) = args.get(1) else {
                usage();
                return 2;
            };
            let mut it = Interp::new();
            if run_source(&mut it, code, false) { 0 } else { 1 }
        }
        Some(path) => {
            let src = match std::fs::read(path) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
                    Err(_) => {
                        eprintln!("⚠ الملف '{path}' ليس بترميز UTF-8");
                        return 1;
                    }
                },
                Err(e) => {
                    eprintln!("⚠ تعذّر فتح الملف '{path}': {e}");
                    return 1;
                }
            };
            let mut it = Interp::new();
            if run_source(&mut it, &src, false) { 0 } else { 1 }
        }
    }
}

fn main() {
    // Run on a big stack so deep (but legal) recursion reaches RecursionError, not a crash
    let code = std::thread::Builder::new()
        .stack_size(STACK_BYTES)
        .spawn(main_inner)
        .expect("thread")
        .join()
        .unwrap_or(1);
    std::process::exit(code);
}
