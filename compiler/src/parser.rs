// Recursive-descent parser: tokens → syntax tree, with Python's grammar and precedence.

use crate::ast::*;
use crate::lexer::{digit_value, err, tokenize, unescape, SyntaxError, Tok, Token};
use crate::value::{Str, Value};
use std::rc::Rc;

type PResult<T> = Result<T, SyntaxError>;

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

pub fn parse_program(src: &str) -> PResult<Vec<Stmt>> {
    let mut p = Parser { toks: tokenize(src, 1)?, pos: 0 };
    let mut body = Vec::new();
    p.skip_newlines();
    while !p.at(&Tok::Eof) {
        if p.at(&Tok::Indent) {
            return Err(SyntaxError { kind: "IndentationError", msg: "إزاحة غير متوقعة".into(), line: p.line() });
        }
        p.statement(&mut body)?;
        p.skip_newlines();
    }
    Ok(body)
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Name(n) => format!("'{n}'"),
        Tok::Kw(k) => format!("'{k}'"),
        Tok::Op(o) => format!("'{o}'"),
        Tok::Int(_) | Tok::Float(_) => "رقم".into(),
        Tok::Str(_) | Tok::FStr(..) => "نص".into(),
        Tok::Newline => "نهاية السطر".into(),
        Tok::Indent => "إزاحة".into(),
        Tok::Dedent => "نهاية الكتلة".into(),
        Tok::Eof => "نهاية البرنامج".into(),
    }
}

fn name(n: &str) -> Expr {
    Expr::Name(n.into(), Var::Unresolved)
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, k: usize) -> &Tok {
        &self.toks[(self.pos + k).min(self.toks.len() - 1)].tok
    }

    fn line(&self) -> u32 {
        self.toks[self.pos].line
    }

    fn at(&self, t: &Tok) -> bool {
        self.peek() == t
    }

    fn at_op(&self, op: &str) -> bool {
        matches!(self.peek(), Tok::Op(o) if *o == op)
    }

    fn at_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Kw(k) if *k == kw)
    }

    fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat_op(&mut self, op: &str) -> bool {
        if self.at_op(op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at_kw(kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn unexpected<T>(&self) -> PResult<T> {
        Err(err(self.line(), format!("صياغة غير صحيحة: لم يُتوقع {}", describe(self.peek()))))
    }

    fn expect_op(&mut self, op: &str) -> PResult<()> {
        if self.eat_op(op) {
            Ok(())
        } else {
            Err(err(self.line(), format!("كان متوقعا '{op}' لكن وُجد {}", describe(self.peek()))))
        }
    }

    fn expect_name(&mut self) -> PResult<Str> {
        match self.next() {
            Tok::Name(n) => Ok(n),
            t => {
                self.pos -= 1;
                Err(err(self.line(), format!("كان متوقعا اسم لكن وُجد {}", describe(&t))))
            }
        }
    }

    fn skip_newlines(&mut self) {
        while self.at(&Tok::Newline) {
            self.pos += 1;
        }
    }

    // ---------- Statements ----------

    fn statement(&mut self, out: &mut Vec<Stmt>) -> PResult<()> {
        let line = self.line();
        let kind = match self.peek() {
            Tok::Kw("if") => self.if_stmt()?,
            Tok::Kw("while") => {
                self.pos += 1;
                let cond = self.test()?;
                let body = self.block()?;
                let orelse = self.else_block()?;
                StmtKind::While(cond, body, orelse)
            }
            Tok::Kw("for") => {
                self.pos += 1;
                let target = self.target_list()?;
                if !self.eat_kw("in") {
                    return Err(err(self.line(), "كان متوقعا 'في' بعد متغير الحلقة"));
                }
                let iter = self.test_list()?;
                let body = self.block()?;
                let orelse = self.else_block()?;
                StmtKind::For(target, iter, body, orelse)
            }
            Tok::Kw("def") => {
                self.pos += 1;
                let n = self.expect_name()?;
                self.expect_op("(")?;
                let mut f = self.params(")", line)?;
                self.expect_op(")")?;
                if self.eat_op("->") {
                    self.test()?;
                }
                f.name = n.clone();
                f.body = self.block()?;
                StmtKind::Def(Rc::new(f), name(&n))
            }
            Tok::Kw("class") => {
                self.pos += 1;
                let n = self.expect_name()?;
                let mut base = None;
                if self.eat_op("(") {
                    if !self.at_op(")") {
                        base = Some(self.test()?);
                    }
                    self.expect_op(")")?;
                }
                let body = self.block()?;
                StmtKind::Class { name: n.clone(), target: name(&n), base, body }
            }
            Tok::Kw("try") => self.try_stmt()?,
            Tok::Kw("with") => return Err(err(line, "'مع' غير مدعومة بعد")),
            _ => return self.simple_line(out),
        };
        out.push(Stmt { line, kind });
        Ok(())
    }

    /// One line of simple statements separated by ';'
    fn simple_line(&mut self, out: &mut Vec<Stmt>) -> PResult<()> {
        loop {
            let line = self.line();
            let kind = self.simple_stmt()?;
            out.push(Stmt { line, kind });
            if !self.eat_op(";") || self.at(&Tok::Newline) {
                break;
            }
        }
        match self.peek() {
            Tok::Newline => {
                self.pos += 1;
                Ok(())
            }
            Tok::Eof | Tok::Dedent => Ok(()),
            _ => self.unexpected(),
        }
    }

    fn simple_stmt(&mut self) -> PResult<StmtKind> {
        Ok(match self.peek() {
            Tok::Kw("pass") => { self.pos += 1; StmtKind::Pass }
            Tok::Kw("break") => { self.pos += 1; StmtKind::Break }
            Tok::Kw("continue") => { self.pos += 1; StmtKind::Continue }
            Tok::Kw("return") => {
                self.pos += 1;
                StmtKind::Return(if self.at_stmt_end() { None } else { Some(self.test_list()?) })
            }
            Tok::Kw("raise") => {
                self.pos += 1;
                StmtKind::Raise(if self.at_stmt_end() { None } else { Some(self.test()?) })
            }
            Tok::Kw("global") | Tok::Kw("nonlocal") => {
                let is_global = self.at_kw("global");
                self.pos += 1;
                let mut names = vec![self.expect_name()?];
                while self.eat_op(",") {
                    names.push(self.expect_name()?);
                }
                if is_global { StmtKind::Global(names) } else { StmtKind::Nonlocal(names) }
            }
            Tok::Kw("del") => {
                self.pos += 1;
                let mut targets = vec![self.or_test()?];
                while self.eat_op(",") {
                    targets.push(self.or_test()?);
                }
                StmtKind::Del(targets)
            }
            Tok::Kw("assert") => {
                self.pos += 1;
                let cond = self.test()?;
                let msg = if self.eat_op(",") { Some(self.test()?) } else { None };
                StmtKind::Assert(cond, msg)
            }
            Tok::Kw("import") => {
                self.pos += 1;
                let mut mods = Vec::new();
                loop {
                    let m = self.expect_name()?;
                    let alias = if self.eat_kw("as") { self.expect_name()? } else { m.clone() };
                    mods.push((m, name(&alias)));
                    if !self.eat_op(",") {
                        break;
                    }
                }
                StmtKind::Import(mods)
            }
            Tok::Kw("from") => {
                self.pos += 1;
                let m = self.expect_name()?;
                if !self.eat_kw("import") {
                    return Err(err(self.line(), "كان متوقعا 'استيراد'"));
                }
                let paren = self.eat_op("(");
                let mut names = Vec::new();
                loop {
                    let n = self.expect_name()?;
                    let alias = if self.eat_kw("as") { self.expect_name()? } else { n.clone() };
                    names.push((n, name(&alias)));
                    if !self.eat_op(",") || (paren && self.at_op(")")) {
                        break;
                    }
                }
                if paren {
                    self.expect_op(")")?;
                }
                StmtKind::ImportFrom(m, names)
            }
            _ => {
                let first = self.test_list()?;
                if let Tok::Op(op) = self.peek() {
                    let aug = match *op {
                        "+=" => Some(BinOp::Add), "-=" => Some(BinOp::Sub), "*=" => Some(BinOp::Mul),
                        "/=" => Some(BinOp::Div), "//=" => Some(BinOp::FloorDiv), "%=" => Some(BinOp::Mod),
                        "**=" => Some(BinOp::Pow), "&=" => Some(BinOp::BitAnd), "|=" => Some(BinOp::BitOr),
                        "^=" => Some(BinOp::BitXor), "<<=" => Some(BinOp::Shl), ">>=" => Some(BinOp::Shr),
                        _ => None,
                    };
                    if let Some(op) = aug {
                        self.pos += 1;
                        self.check_target(&first, true)?;
                        let value = self.test_list()?;
                        return Ok(StmtKind::AugAssign(first, op, value));
                    }
                }
                if self.at_op("=") {
                    let mut targets = vec![first];
                    let mut value;
                    loop {
                        self.expect_op("=")?;
                        value = self.test_list()?;
                        if !self.at_op("=") {
                            break;
                        }
                        targets.push(value);
                    }
                    for t in &targets {
                        self.check_target(t, false)?;
                    }
                    StmtKind::Assign(targets, value)
                } else {
                    StmtKind::Expr(first)
                }
            }
        })
    }

    fn at_stmt_end(&self) -> bool {
        matches!(self.peek(), Tok::Newline | Tok::Eof | Tok::Dedent) || self.at_op(";")
    }

    fn check_target(&self, e: &Expr, single: bool) -> PResult<()> {
        match e {
            Expr::Name(..) | Expr::Attr(..) | Expr::Index(..) => Ok(()),
            Expr::Tuple(items) | Expr::List(items) if !single => {
                items.iter().try_for_each(|i| self.check_target(i, false))
            }
            _ => Err(err(self.line(), "لا يمكن الإسناد إلى هذا التعبير")),
        }
    }

    fn if_stmt(&mut self) -> PResult<StmtKind> {
        self.pos += 1; // if
        let mut branches = vec![(self.test()?, self.block()?)];
        let mut orelse = Vec::new();
        loop {
            if self.eat_kw("elif") {
                branches.push((self.test()?, self.block()?));
            } else if self.eat_kw("else") {
                orelse = self.block()?;
                break;
            } else {
                break;
            }
        }
        Ok(StmtKind::If(branches, orelse))
    }

    fn else_block(&mut self) -> PResult<Vec<Stmt>> {
        if self.eat_kw("else") { self.block() } else { Ok(Vec::new()) }
    }

    fn try_stmt(&mut self) -> PResult<StmtKind> {
        self.pos += 1; // try
        let body = self.block()?;
        let mut handlers = Vec::new();
        while self.eat_kw("except") {
            let typ = if self.at_op(":") { None } else { Some(self.test()?) };
            let name = if self.eat_kw("as") { Some(name(&self.expect_name()?)) } else { None };
            handlers.push(Handler { typ, name, body: self.block()? });
        }
        let orelse = if !handlers.is_empty() { self.else_block()? } else { Vec::new() };
        let finally = if self.eat_kw("finally") { self.block()? } else { Vec::new() };
        if handlers.is_empty() && finally.is_empty() {
            return Err(err(self.line(), "'حاول' تحتاج 'عدا' أو 'أخيرا'"));
        }
        Ok(StmtKind::Try { body, handlers, orelse, finally })
    }

    /// ':' followed by an indented block, or simple statements on the same line
    fn block(&mut self) -> PResult<Vec<Stmt>> {
        if !self.eat_op(":") {
            return Err(err(self.line(), format!("كان متوقعا ':' لكن وُجد {}", describe(self.peek()))));
        }
        let mut body = Vec::new();
        if !self.at(&Tok::Newline) {
            self.simple_line(&mut body)?;
            return Ok(body);
        }
        self.skip_newlines();
        if !self.at(&Tok::Indent) {
            return Err(SyntaxError { kind: "IndentationError", msg: "كان متوقعا كتلة مُزاحة بعد ':'".into(), line: self.line() });
        }
        self.pos += 1;
        while !self.at(&Tok::Dedent) && !self.at(&Tok::Eof) {
            self.statement(&mut body)?;
            self.skip_newlines();
        }
        self.eat_dedent();
        Ok(body)
    }

    fn eat_dedent(&mut self) {
        if self.at(&Tok::Dedent) {
            self.pos += 1;
        }
    }

    /// Parameters up to (not including) `end`; returns a FuncDef with an empty body
    fn params(&mut self, end: &str, line: u32) -> PResult<FuncDef> {
        let mut f = FuncDef {
            name: "<lambda>".into(), params: Vec::new(), vararg: None, kwarg: None, body: Vec::new(),
            line, nlocals: 0, simple_params: false, ncells: 0, freevars: Vec::new(),
        };
        while !self.at_op(end) {
            if self.eat_op("**") {
                f.kwarg = Some((self.expect_name()?, Var::Unresolved));
            } else if self.eat_op("*") {
                f.vararg = Some((self.expect_name()?, Var::Unresolved));
            } else {
                let n = self.expect_name()?;
                if end == ")" && self.eat_op(":") {
                    self.test()?; // type annotation, ignored
                }
                let default = if self.eat_op("=") { Some(self.test()?) } else { None };
                if default.is_none() && f.params.last().is_some_and(|p| p.default.is_some()) {
                    return Err(err(self.line(), "معامل بلا قيمة افتراضية بعد معامل له قيمة افتراضية"));
                }
                f.params.push(Param { name: n, var: Var::Unresolved, default });
            }
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(f)
    }

    // ---------- Expressions ----------

    /// Comma-separated expressions; a tuple if there is more than one (or a trailing comma)
    fn test_list(&mut self) -> PResult<Expr> {
        let first = self.test()?;
        if !self.at_op(",") {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_op(",") {
            if self.at_stmt_end() || self.at_op("=") || self.at_op(")") || matches!(self.peek(), Tok::Op(o) if o.ends_with('=') && *o != "==") {
                break;
            }
            items.push(self.test()?);
        }
        Ok(Expr::Tuple(items))
    }

    /// Loop targets: stops before 'in'
    fn target_list(&mut self) -> PResult<Expr> {
        let first = self.bit_or()?;
        let t = if self.at_op(",") {
            let mut items = vec![first];
            while self.eat_op(",") {
                if self.at_kw("in") {
                    break;
                }
                items.push(self.bit_or()?);
            }
            Expr::Tuple(items)
        } else {
            first
        };
        self.check_target(&t, false)?;
        Ok(t)
    }

    pub fn test(&mut self) -> PResult<Expr> {
        if self.eat_kw("lambda") {
            let line = self.line();
            let mut f = self.params(":", line)?;
            self.expect_op(":")?;
            let body = self.test()?;
            f.body = vec![Stmt { line, kind: StmtKind::Return(Some(body)) }];
            return Ok(Expr::Lambda(Rc::new(f)));
        }
        let e = self.or_test()?;
        if self.eat_kw("if") {
            let cond = self.or_test()?;
            if !self.eat_kw("else") {
                return Err(err(self.line(), "كان متوقعا 'وإلا' في التعبير الشرطي"));
            }
            let orelse = self.test()?;
            return Ok(Expr::IfExp(Box::new(e), Box::new(cond), Box::new(orelse)));
        }
        Ok(e)
    }

    fn or_test(&mut self) -> PResult<Expr> {
        let mut e = self.and_test()?;
        while self.eat_kw("or") {
            e = Expr::Or(Box::new(e), Box::new(self.and_test()?));
        }
        Ok(e)
    }

    fn and_test(&mut self) -> PResult<Expr> {
        let mut e = self.not_test()?;
        while self.eat_kw("and") {
            e = Expr::And(Box::new(e), Box::new(self.not_test()?));
        }
        Ok(e)
    }

    fn not_test(&mut self) -> PResult<Expr> {
        if self.eat_kw("not") {
            return Ok(Expr::Unary(UnOp::Not, Box::new(self.not_test()?)));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> PResult<Expr> {
        let first = self.bit_or()?;
        let mut ops = Vec::new();
        loop {
            let op = match self.peek() {
                Tok::Op("==") => CmpOp::Eq,
                Tok::Op("!=") => CmpOp::Ne,
                Tok::Op("<") => CmpOp::Lt,
                Tok::Op("<=") => CmpOp::Le,
                Tok::Op(">") => CmpOp::Gt,
                Tok::Op(">=") => CmpOp::Ge,
                Tok::Kw("in") => CmpOp::In,
                Tok::Kw("not") if matches!(self.peek_at(1), Tok::Kw("in")) => {
                    self.pos += 1;
                    CmpOp::NotIn
                }
                Tok::Kw("is") => {
                    if matches!(self.peek_at(1), Tok::Kw("not")) {
                        self.pos += 1;
                        CmpOp::IsNot
                    } else {
                        CmpOp::Is
                    }
                }
                _ => break,
            };
            self.pos += 1;
            ops.push((op, self.bit_or()?));
        }
        Ok(if ops.is_empty() { first } else { Expr::Compare(Box::new(first), ops) })
    }

    fn binary(&mut self, level: usize) -> PResult<Expr> {
        const LEVELS: &[&[(&str, BinOp)]] = &[
            &[("|", BinOp::BitOr)],
            &[("^", BinOp::BitXor)],
            &[("&", BinOp::BitAnd)],
            &[("<<", BinOp::Shl), (">>", BinOp::Shr)],
            &[("+", BinOp::Add), ("-", BinOp::Sub)],
            &[("*", BinOp::Mul), ("/", BinOp::Div), ("//", BinOp::FloorDiv), ("%", BinOp::Mod)],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut e = self.binary(level + 1)?;
        'outer: loop {
            for (sym, op) in LEVELS[level] {
                if self.eat_op(sym) {
                    e = Expr::Bin(*op, Box::new(e), Box::new(self.binary(level + 1)?));
                    continue 'outer;
                }
            }
            return Ok(e);
        }
    }

    fn bit_or(&mut self) -> PResult<Expr> {
        self.binary(0)
    }

    fn unary(&mut self) -> PResult<Expr> {
        let op = match self.peek() {
            Tok::Op("-") => UnOp::Neg,
            Tok::Op("+") => UnOp::Pos,
            Tok::Op("~") => UnOp::Invert,
            _ => return self.power(),
        };
        self.pos += 1;
        let operand = self.unary()?;
        // Fold negative literals so -5 is a constant
        Ok(match (op, operand) {
            (UnOp::Neg, Expr::Const(Value::Int(i))) => Expr::Const(Value::Int(-i)),
            (UnOp::Neg, Expr::Const(Value::Float(f))) => Expr::Const(Value::Float(-f)),
            (op, e) => Expr::Unary(op, Box::new(e)),
        })
    }

    fn power(&mut self) -> PResult<Expr> {
        let base = self.primary()?;
        if self.eat_op("**") {
            let exp = self.unary()?; // right-associative, binds tighter than unary on the left
            return Ok(Expr::Bin(BinOp::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> PResult<Expr> {
        let mut e = self.atom()?;
        loop {
            if self.eat_op("(") {
                let args = self.call_args()?;
                e = Expr::Call(Box::new(e), args);
            } else if self.eat_op("[") {
                let idx = self.subscript()?;
                self.expect_op("]")?;
                e = Expr::Index(Box::new(e), Box::new(idx));
            } else if self.eat_op(".") {
                let attr = match self.next() {
                    Tok::Name(n) => n,
                    // Arabic keywords can't be attribute names, but English words like 'print' can
                    Tok::Kw(k) => k.into(),
                    _ => return Err(err(self.line(), "كان متوقعا اسم بعد '.'")),
                };
                e = Expr::Attr(Box::new(e), attr);
            } else {
                return Ok(e);
            }
        }
    }

    fn call_args(&mut self) -> PResult<Vec<Arg>> {
        let mut args = Vec::new();
        while !self.at_op(")") {
            if self.eat_op("**") {
                args.push(Arg::StarStar(self.test()?));
            } else if self.eat_op("*") {
                args.push(Arg::Star(self.test()?));
            } else if matches!(self.peek(), Tok::Name(_)) && matches!(self.peek_at(1), Tok::Op("=")) {
                let n = self.expect_name()?;
                self.pos += 1;
                args.push(Arg::Kw(n, self.test()?));
            } else {
                let e = self.test()?;
                if self.at_kw("for") {
                    // Generator expression argument: sum(x for x in xs)
                    let comps = self.comp_for()?;
                    args.push(Arg::Pos(Expr::ListComp(Box::new(e), comps)));
                } else {
                    args.push(Arg::Pos(e));
                }
            }
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op(")")?;
        Ok(args)
    }

    fn subscript(&mut self) -> PResult<Expr> {
        let part = |p: &mut Parser| -> PResult<Option<Box<Expr>>> {
            Ok(if p.at_op(":") || p.at_op("]") { None } else { Some(Box::new(p.test()?)) })
        };
        let start = part(self)?;
        if !self.eat_op(":") {
            let first = *start.unwrap_or_else(|| Box::new(Expr::Const(Value::None)));
            if self.at_op(",") {
                let mut items = vec![first];
                while self.eat_op(",") && !self.at_op("]") {
                    items.push(self.test()?);
                }
                return Ok(Expr::Tuple(items));
            }
            return Ok(first);
        }
        let stop = part(self)?;
        let step = if self.eat_op(":") { part(self)? } else { None };
        Ok(Expr::Slice(start, stop, step))
    }

    fn comp_for(&mut self) -> PResult<Vec<Comp>> {
        let mut comps = Vec::new();
        while self.eat_kw("for") {
            let target = self.target_list()?;
            if !self.eat_kw("in") {
                return Err(err(self.line(), "كان متوقعا 'في'"));
            }
            let iter = self.or_test()?;
            let mut conds = Vec::new();
            while self.eat_kw("if") {
                conds.push(self.or_test()?);
            }
            comps.push(Comp { target, iter, conds });
        }
        Ok(comps)
    }

    fn atom(&mut self) -> PResult<Expr> {
        let line = self.line();
        Ok(match self.next() {
            Tok::Name(n) => Expr::Name(n, Var::Unresolved),
            Tok::Int(i) => Expr::Const(Value::Int(i)),
            Tok::Float(f) => Expr::Const(Value::Float(f)),
            Tok::Kw("True") => Expr::Const(Value::Bool(true)),
            Tok::Kw("False") => Expr::Const(Value::Bool(false)),
            Tok::Kw("None") => Expr::Const(Value::None),
            Tok::Str(_) | Tok::FStr(..) => {
                self.pos -= 1;
                self.strings()?
            }
            Tok::Op("(") => {
                if self.eat_op(")") {
                    return Ok(Expr::Tuple(Vec::new()));
                }
                let first = self.test()?;
                if self.at_kw("for") {
                    let comps = self.comp_for()?;
                    self.expect_op(")")?;
                    return Ok(Expr::ListComp(Box::new(first), comps));
                }
                if !self.at_op(",") {
                    self.expect_op(")")?;
                    return Ok(first);
                }
                let mut items = vec![first];
                while self.eat_op(",") && !self.at_op(")") {
                    items.push(self.test()?);
                }
                self.expect_op(")")?;
                Expr::Tuple(items)
            }
            Tok::Op("[") => {
                if self.eat_op("]") {
                    return Ok(Expr::List(Vec::new()));
                }
                let first = self.test()?;
                if self.at_kw("for") {
                    let comps = self.comp_for()?;
                    self.expect_op("]")?;
                    return Ok(Expr::ListComp(Box::new(first), comps));
                }
                let mut items = vec![first];
                while self.eat_op(",") && !self.at_op("]") {
                    items.push(self.test()?);
                }
                self.expect_op("]")?;
                Expr::List(items)
            }
            Tok::Op("{") => {
                if self.eat_op("}") {
                    return Ok(Expr::Dict(Vec::new()));
                }
                let k = self.test()?;
                if !self.eat_op(":") {
                    return Err(err(line, "المجموعات {أ، ب} غير مدعومة؛ استخدم قائمة أو قاموسا"));
                }
                let v = self.test()?;
                if self.at_kw("for") {
                    let comps = self.comp_for()?;
                    self.expect_op("}")?;
                    return Ok(Expr::DictComp(Box::new((k, v)), comps));
                }
                let mut items = vec![(k, v)];
                while self.eat_op(",") && !self.at_op("}") {
                    let k = self.test()?;
                    self.expect_op(":")?;
                    items.push((k, self.test()?));
                }
                self.expect_op("}")?;
                Expr::Dict(items)
            }
            _ => {
                self.pos -= 1;
                return self.unexpected();
            }
        })
    }

    /// Adjacent string literals concatenate: "أ" "ب" == "أب"
    fn strings(&mut self) -> PResult<Expr> {
        let mut parts: Vec<FPart> = Vec::new();
        let mut is_f = false;
        loop {
            let line = self.line();
            match self.peek().clone() {
                Tok::Str(s) => {
                    self.pos += 1;
                    parts.push(FPart::Lit(s));
                }
                Tok::FStr(s, raw) => {
                    self.pos += 1;
                    is_f = true;
                    parse_fstring(&s, raw, line, &mut parts)?;
                }
                _ => break,
            }
        }
        if !is_f {
            let s: String = parts.iter().map(|p| match p { FPart::Lit(s) => &**s, _ => "" }).collect();
            return Ok(Expr::Const(Value::Str(s.into())));
        }
        Ok(Expr::FString(parts))
    }
}

/// Splits an f-string into literal text and {expression!conv:spec} parts
fn parse_fstring(s: &str, raw: bool, line: u32, parts: &mut Vec<FPart>) -> PResult<()> {
    let chars: Vec<char> = s.chars().collect();
    let mut lit = String::new();
    let mut i = 0;
    let flush = |lit: &mut String, parts: &mut Vec<FPart>| {
        if !lit.is_empty() {
            let text = if raw { lit.clone() } else { unescape(lit) };
            parts.push(FPart::Lit(text.into()));
            lit.clear();
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '{' && chars.get(i + 1) == Some(&'{') {
            lit.push('{');
            i += 2;
        } else if c == '}' && chars.get(i + 1) == Some(&'}') {
            lit.push('}');
            i += 2;
        } else if c == '{' {
            flush(&mut lit, parts);
            // Find the end of the expression, skipping nested brackets and strings
            let start = i + 1;
            let mut depth = 0;
            let mut j = start;
            let mut quote: Option<char> = None;
            let mut conv_at = None;
            let mut spec_at = None;
            while j < chars.len() {
                let ch = chars[j];
                if let Some(q) = quote {
                    if ch == q {
                        quote = None;
                    }
                } else if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                } else if "([{".contains(ch) {
                    depth += 1;
                } else if ")]".contains(ch) || (ch == '}' && depth > 0) {
                    depth -= 1;
                } else if ch == '}' {
                    break;
                } else if depth == 0 && ch == '!' && chars.get(j + 1) != Some(&'=') && spec_at.is_none() {
                    conv_at = Some(j);
                } else if depth == 0 && ch == ':' && spec_at.is_none() {
                    spec_at = Some(j);
                }
                j += 1;
            }
            if j >= chars.len() {
                return Err(err(line, "قوس '{' غير مغلق في النص المنسق"));
            }
            let expr_end = conv_at.or(spec_at).unwrap_or(j);
            let src: String = chars[start..expr_end].iter().collect();
            let conv = conv_at.and_then(|k| chars.get(k + 1).copied());
            // Arabic digits in the spec ({x:.٢f}) mean the same as ASCII ones
            let spec: String = spec_at.map(|k| chars[k + 1..j].iter()
                .map(|&c| digit_value(c).and_then(|d| char::from_digit(d, 10)).unwrap_or(c)).collect()).unwrap_or_default();
            if src.trim().is_empty() {
                return Err(err(line, "تعبير فارغ داخل {} في النص المنسق"));
            }
            let mut p = Parser { toks: tokenize(&format!("({src})"), line)?, pos: 0 };
            let expr = p.test()?;
            p.skip_newlines();
            if !p.at(&Tok::Eof) {
                return p.unexpected();
            }
            parts.push(FPart::Expr { expr, conv, spec: spec.into() });
            i = j + 1;
        } else if c == '}' {
            return Err(err(line, "قوس '}' منفرد في النص المنسق؛ استخدم '}}'"));
        } else {
            lit.push(c);
            i += 1;
        }
    }
    flush(&mut lit, parts);
    Ok(())
}
