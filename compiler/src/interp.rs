// Tree-walking interpreter over the resolved syntax tree.

use crate::ast::*;
use crate::builtins;
use crate::resolve::Globals;
use crate::value::*;
use std::cell::RefCell;
use std::io::{IsTerminal, Write as _};
use std::rc::Rc;

pub const MAX_DEPTH: usize = 1000;

pub struct Exc {
    pub value: Value,
    pub line: u32,
}

pub type R<T> = Result<T, Box<Exc>>;

/// Integer arithmetic that can't fail or change type; None means take the general path
#[inline(always)]
fn int_fast(op: BinOp, x: i64, y: i64) -> Option<i64> {
    match op {
        BinOp::Add => x.checked_add(y),
        BinOp::Sub => x.checked_sub(y),
        BinOp::Mul => x.checked_mul(y),
        BinOp::Mod if y > 0 && x >= 0 => Some(x % y),
        BinOp::FloorDiv if y > 0 && x >= 0 => Some(x / y),
        _ => None,
    }
}

pub enum Flow {
    Normal,
    Break,
    Continue,
    Return(Value),
}

pub struct Frame {
    pub locals: Vec<Value>,
    pub cells: Vec<Rc<RefCell<Value>>>,
    pub free: Rc<Vec<Rc<RefCell<Value>>>>,
    pub func: Option<Rc<Function>>,
    /// Namespace of the class body being executed, if any
    pub class_ns: Option<AttrMap>,
}

impl Frame {
    pub fn module() -> Frame {
        Frame { locals: Vec::new(), cells: Vec::new(), free: Rc::new(Vec::new()), func: None, class_ns: None }
    }
}

/// Built-in exception classes and their parents
const EXCEPTIONS: &[(&str, Option<&str>)] = &[
    ("BaseException", None),
    ("Exception", Some("BaseException")),
    ("KeyboardInterrupt", Some("BaseException")),
    ("ArithmeticError", Some("Exception")),
    ("ZeroDivisionError", Some("ArithmeticError")),
    ("OverflowError", Some("ArithmeticError")),
    ("LookupError", Some("Exception")),
    ("IndexError", Some("LookupError")),
    ("KeyError", Some("LookupError")),
    ("ValueError", Some("Exception")),
    ("TypeError", Some("Exception")),
    ("NameError", Some("Exception")),
    ("UnboundLocalError", Some("NameError")),
    ("AttributeError", Some("Exception")),
    ("RuntimeError", Some("Exception")),
    ("RecursionError", Some("RuntimeError")),
    ("NotImplementedError", Some("RuntimeError")),
    ("EOFError", Some("Exception")),
    ("AssertionError", Some("Exception")),
    ("ImportError", Some("Exception")),
    ("ModuleNotFoundError", Some("ImportError")),
    ("StopIteration", Some("Exception")),
];

pub struct Interp {
    pub globals: Vec<Value>,
    pub names: Globals,
    builtins: Map<&'static str, Value>,
    pub exceptions: Map<&'static str, Rc<Class>>,
    base_exception: Rc<Class>,
    out: String,
    out_is_terminal: bool,
    depth: usize,
    /// Exceptions currently being handled, for bare `raise`
    handling: Vec<Value>,
    modules: Map<Str, Value>,
    /// Spare vectors for call arguments/locals, to avoid an allocation per call
    vec_pool: Vec<Vec<Value>>,
    init_name: Str,
    pub rng: u64,
}

impl Interp {
    pub fn new() -> Interp {
        let mut builtins: Map<&'static str, Value> = Map::default();
        for b in builtins::GLOBAL {
            builtins.insert(b.name(), Value::Builtin(*b));
        }
        let mut exceptions: Map<&'static str, Rc<Class>> = Map::default();
        for (name, parent) in EXCEPTIONS {
            let cls = Rc::new(Class {
                name: (*name).into(),
                base: parent.map(|p| exceptions[p].clone()),
                attrs: RefCell::new(AttrMap::default()),
            });
            builtins.insert(name, Value::Class(cls.clone()));
            exceptions.insert(name, cls);
        }
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Interp {
            globals: Vec::new(),
            names: Globals::default(),
            builtins,
            base_exception: exceptions["BaseException"].clone(),
            exceptions,
            out: String::new(),
            out_is_terminal: std::io::stdout().is_terminal(),
            depth: 0,
            handling: Vec::new(),
            modules: Map::default(),
            vec_pool: Vec::new(),
            init_name: crate::lexer::intern("__init__"),
            rng: seed | 1,
        }
    }

    /// Gives new global slots their builtin value (if any). Call after resolving.
    pub fn sync_globals(&mut self) {
        while self.globals.len() < self.names.names.len() {
            let name = &self.names.names[self.globals.len()];
            let v = self.builtins.get(&**name).cloned().unwrap_or(Value::Undef);
            self.globals.push(v);
        }
    }

    // ---------- Output ----------

    pub fn write(&mut self, s: &str) {
        self.out.push_str(s);
        if self.out_is_terminal || self.out.len() > 1 << 16 {
            self.flush();
        }
    }

    pub fn flush(&mut self) {
        if !self.out.is_empty() {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(self.out.as_bytes());
            let _ = stdout.flush();
            self.out.clear();
        }
    }

    // ---------- Errors ----------

    pub fn err(&self, kind: &str, msg: impl Into<String>) -> Box<Exc> {
        let cls = self.exceptions[kind].clone();
        let inst = Instance { class: cls, attrs: RefCell::new(AttrMap::default()) };
        inst.attrs.borrow_mut().insert(crate::lexer::intern("args"), Value::tuple(vec![Value::Str(msg.into().into())]));
        Box::new(Exc { value: Value::Instance(Rc::new(inst)), line: 0 })
    }

    pub fn type_err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(self.err("TypeError", msg))
    }

    pub fn is_exception_class(&self, c: &Rc<Class>) -> bool {
        c.is_subclass_of(&self.base_exception)
    }

    // ---------- Running ----------

    pub fn run(&mut self, body: &[Stmt]) -> R<()> {
        let mut frame = Frame::module();
        self.exec_block(&mut frame, body).map(|_| ())
    }

    pub fn exec_block(&mut self, f: &mut Frame, body: &[Stmt]) -> R<Flow> {
        for s in body {
            match self.exec(f, s) {
                Ok(Flow::Normal) => {}
                Err(mut e) => {
                    if e.line == 0 {
                        e.line = s.line;
                    }
                    return Err(e);
                }
                other => return other,
            }
        }
        Ok(Flow::Normal)
    }

    fn exec(&mut self, f: &mut Frame, s: &Stmt) -> R<Flow> {
        match &s.kind {
            StmtKind::Expr(e) => {
                self.eval(f, e)?;
            }
            StmtKind::Assign(targets, value) => {
                let v = self.eval(f, value)?;
                if let [Expr::Name(n, var)] = targets.as_slice() {
                    self.store(f, n, *var, v);
                } else {
                    for t in targets {
                        self.assign(f, t, v.clone())?;
                    }
                }
            }
            StmtKind::AugAssign(target, op, value) => self.aug_assign(f, target, *op, value)?,
            StmtKind::If(branches, orelse) => {
                for (cond, body) in branches {
                    if self.eval(f, cond)?.truthy() {
                        return self.exec_block(f, body);
                    }
                }
                return self.exec_block(f, orelse);
            }
            StmtKind::While(cond, body, orelse) => {
                while self.eval(f, cond)?.truthy() {
                    match self.exec_block(f, body)? {
                        Flow::Break => return Ok(Flow::Normal),
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
                return self.exec_block(f, orelse);
            }
            StmtKind::For(target, iter, body, orelse) => return self.for_loop(f, target, iter, body, orelse),
            StmtKind::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(f, e)?,
                    None => Value::None,
                };
                return Ok(Flow::Return(v));
            }
            StmtKind::Pass | StmtKind::Global(_) | StmtKind::Nonlocal(_) => {}
            StmtKind::Break => return Ok(Flow::Break),
            StmtKind::Continue => return Ok(Flow::Continue),
            _ => return self.exec_cold(f, s),
        }
        Ok(Flow::Normal)
    }

    /// Less common statements, kept out of `exec` so its stack frame stays small
    #[inline(never)]
    fn exec_cold(&mut self, f: &mut Frame, s: &Stmt) -> R<Flow> {
        match &s.kind {
            StmtKind::Def(def, target) => {
                let func = self.make_function(f, def)?;
                self.assign(f, target, func)?;
            }
            StmtKind::Class { name, target, base, body } => {
                let base = match base {
                    Some(e) => match self.eval(f, e)? {
                        Value::Class(c) => Some(c),
                        other => return self.type_err(format!("لا يمكن الوراثة من {}", other.type_name())),
                    },
                    None => None,
                };
                let saved = f.class_ns.replace(AttrMap::default());
                let r = self.exec_block(f, body);
                let ns = std::mem::replace(&mut f.class_ns, saved).unwrap();
                r?;
                let cls = Rc::new(Class { name: name.clone(), base, attrs: RefCell::new(ns) });
                for v in cls.attrs.borrow().values() {
                    if let Value::Func(func) = v {
                        *func.owner.borrow_mut() = Some(cls.clone());
                    }
                }
                self.assign(f, target, Value::Class(cls))?;
            }
            StmtKind::Raise(e) => {
                let value = match e {
                    None => match self.handling.last() {
                        Some(v) => v.clone(),
                        None => return Err(self.err("RuntimeError", "لا يوجد خطأ نشط لإعادة رفعه")),
                    },
                    Some(e) => {
                        let v = self.eval(f, e)?;
                        self.make_exception(v)?
                    }
                };
                return Err(Box::new(Exc { value, line: 0 }));
            }
            StmtKind::Try { body, handlers, orelse, finally } => {
                let result = match self.exec_block(f, body) {
                    Err(exc) => self.handle(f, exc, handlers),
                    Ok(Flow::Normal) => self.exec_block(f, orelse),
                    other => other,
                };
                if finally.is_empty() {
                    return result;
                }
                return match self.exec_block(f, finally) {
                    Ok(Flow::Normal) => result,
                    other => other,
                };
            }
            StmtKind::Del(targets) => {
                for t in targets {
                    self.delete(f, t)?;
                }
            }
            StmtKind::Assert(cond, msg) => {
                if !self.eval(f, cond)?.truthy() {
                    let m = match msg {
                        Some(m) => {
                            let v = self.eval(f, m)?;
                            self.to_str(&v)?
                        }
                        None => String::new(),
                    };
                    return Err(self.err("AssertionError", m));
                }
            }
            StmtKind::Import(items) => {
                for (module, target) in items {
                    let m = self.import(module)?;
                    self.assign(f, target, m)?;
                }
            }
            StmtKind::ImportFrom(module, items) => {
                let m = self.import(module)?;
                for (n, target) in items {
                    let v = self.getattr(&m, n)?;
                    self.assign(f, target, v)?;
                }
            }
            _ => unreachable!("handled in exec"),
        }
        Ok(Flow::Normal)
    }

    fn handle(&mut self, f: &mut Frame, exc: Box<Exc>, handlers: &[Handler]) -> R<Flow> {
        for h in handlers {
            let matches = match &h.typ {
                None => true,
                Some(t) => {
                    let t = self.eval(f, t)?;
                    self.exception_matches(&exc.value, &t)?
                }
            };
            if matches {
                if let Some(n) = &h.name {
                    self.assign(f, n, exc.value.clone())?;
                }
                self.handling.push(exc.value.clone());
                let r = self.exec_block(f, &h.body);
                self.handling.pop();
                return r;
            }
        }
        Err(exc)
    }

    fn exception_matches(&self, exc: &Value, typ: &Value) -> R<bool> {
        match typ {
            Value::Class(c) => Ok(matches!(exc, Value::Instance(i) if i.class.is_subclass_of(c))),
            Value::Tuple(items) => {
                for t in items.iter() {
                    if self.exception_matches(exc, t)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => self.type_err("ما بعد 'عدا' يجب أن يكون صنف خطأ"),
        }
    }

    /// `raise X`: X may be an exception class or instance
    fn make_exception(&mut self, v: Value) -> R<Value> {
        match &v {
            Value::Class(c) if self.is_exception_class(c) => self.call(&v, Vec::new(), Vec::new()),
            Value::Instance(i) if self.is_exception_class(&i.class) => Ok(v),
            _ => self.type_err("يمكن رفع أصناف الأخطاء فقط (مثل ValueError)"),
        }
    }

    fn for_loop(&mut self, f: &mut Frame, target: &Expr, iter: &Expr, body: &[Stmt], orelse: &[Stmt]) -> R<Flow> {
        macro_rules! step {
            ($item:expr) => {{
                match target {
                    Expr::Name(n, var) => self.store(f, n, *var, $item),
                    _ => self.assign(f, target, $item)?,
                }
                match self.exec_block(f, body)? {
                    Flow::Break => return Ok(Flow::Normal),
                    Flow::Return(v) => return Ok(Flow::Return(v)),
                    _ => {}
                }
            }};
        }
        let it = self.eval(f, iter)?;
        match &it {
            Value::Range(r) => {
                let (mut i, stop, st) = **r;
                while (st > 0 && i < stop) || (st < 0 && i > stop) {
                    step!(Value::Int(i));
                    i += st;
                }
            }
            Value::List(l) => {
                // By index, so appending while looping behaves like Python
                let mut i = 0;
                loop {
                    let item = match l.borrow().get(i) {
                        Some(v) => v.clone(),
                        None => break,
                    };
                    i += 1;
                    step!(item);
                }
            }
            _ => {
                for item in self.iterate(&it)? {
                    step!(item);
                }
            }
        }
        self.exec_block(f, orelse)
    }

    // ---------- Variables ----------

    #[inline]
    fn load(&self, f: &Frame, name: &Str, var: Var) -> R<Value> {
        let v = match var {
            Var::Local(i) => f.locals[i as usize].clone(),
            Var::Cell(i) => f.cells[i as usize].borrow().clone(),
            Var::Free(i) => f.free[i as usize].borrow().clone(),
            Var::Global(i) => self.globals[i as usize].clone(),
            Var::ClassNs(i) => match f.class_ns.as_ref().and_then(|ns| ns.get(name)) {
                Some(v) => v.clone(),
                None => self.globals[i as usize].clone(),
            },
            Var::Unresolved => unreachable!("unresolved name {name}"),
        };
        if let Value::Undef = v {
            return Err(match var {
                Var::Local(_) | Var::Cell(_) => self.err("UnboundLocalError", format!("المتغير '{name}' استُخدم قبل أن تُسند له قيمة")),
                _ => self.err("NameError", format!("الاسم '{name}' غير معرّف")),
            });
        }
        Ok(v)
    }

    #[inline]
    fn store(&mut self, f: &mut Frame, name: &Str, var: Var, v: Value) {
        match var {
            Var::Local(i) => f.locals[i as usize] = v,
            Var::Cell(i) => *f.cells[i as usize].borrow_mut() = v,
            Var::Free(i) => *f.free[i as usize].borrow_mut() = v,
            Var::Global(i) => self.globals[i as usize] = v,
            Var::ClassNs(i) => match f.class_ns.as_mut() {
                Some(ns) => {
                    ns.insert(name.clone(), v);
                }
                None => self.globals[i as usize] = v,
            },
            Var::Unresolved => unreachable!(),
        }
    }

    pub fn assign(&mut self, f: &mut Frame, target: &Expr, v: Value) -> R<()> {
        match target {
            Expr::Name(n, var) => {
                self.store(f, n, *var, v);
                Ok(())
            }
            Expr::Attr(obj, name) => {
                let o = self.eval(f, obj)?;
                self.setattr(&o, name, v)
            }
            Expr::Index(obj, idx) => {
                let o = self.eval(f, obj)?;
                if let Expr::Slice(a, b, c) = &**idx {
                    let (a, b, c) = self.slice_parts(f, a, b, c)?;
                    return self.set_slice(&o, a, b, c, v);
                }
                let i = self.eval(f, idx)?;
                self.setitem(&o, i, v)
            }
            Expr::Tuple(items) | Expr::List(items) => {
                let values = match &v {
                    Value::Tuple(t) => t.to_vec(),
                    _ => self.iterate(&v)?,
                };
                if values.len() != items.len() {
                    return Err(self.err("ValueError", format!(
                        "عدد القيم ({}) لا يساوي عدد المتغيرات ({})", values.len(), items.len())));
                }
                for (t, v) in items.iter().zip(values) {
                    self.assign(f, t, v)?;
                }
                Ok(())
            }
            _ => self.type_err("لا يمكن الإسناد إلى هذا التعبير"),
        }
    }

    fn aug_assign(&mut self, f: &mut Frame, target: &Expr, op: BinOp, value: &Expr) -> R<()> {
        match target {
            Expr::Name(n, var) => {
                let cur = self.load(f, n, *var)?;
                let rhs = self.eval(f, value)?;
                if let (Value::Int(x), Value::Int(y)) = (&cur, &rhs) {
                    if let Some(r) = int_fast(op, *x, *y) {
                        self.store(f, n, *var, Value::Int(r));
                        return Ok(());
                    }
                }
                let v = self.aug_op(op, cur, rhs)?;
                self.store(f, n, *var, v);
            }
            Expr::Attr(obj, name) => {
                let o = self.eval(f, obj)?;
                let cur = self.getattr(&o, name)?;
                let rhs = self.eval(f, value)?;
                let v = self.aug_op(op, cur, rhs)?;
                self.setattr(&o, name, v)?;
            }
            Expr::Index(obj, idx) => {
                let o = self.eval(f, obj)?;
                let i = self.eval(f, idx)?;
                let cur = self.getitem(&o, &i)?;
                let rhs = self.eval(f, value)?;
                let v = self.aug_op(op, cur, rhs)?;
                self.setitem(&o, i, v)?;
            }
            _ => return self.type_err("لا يمكن الإسناد إلى هذا التعبير"),
        }
        Ok(())
    }

    /// `+=` on a list extends it in place, like Python
    fn aug_op(&mut self, op: BinOp, cur: Value, rhs: Value) -> R<Value> {
        if let (BinOp::Add, Value::List(l)) = (op, &cur) {
            let items = self.iterate(&rhs)?;
            l.borrow_mut().extend(items);
            return Ok(cur);
        }
        self.binop(op, cur, rhs)
    }

    fn delete(&mut self, f: &mut Frame, target: &Expr) -> R<()> {
        match target {
            Expr::Name(n, var) => {
                self.load(f, n, *var)?;
                match var {
                    Var::ClassNs(_) if f.class_ns.is_some() => {
                        f.class_ns.as_mut().unwrap().remove(n);
                    }
                    _ => self.store(f, n, *var, Value::Undef),
                }
                Ok(())
            }
            Expr::Index(obj, idx) => {
                let o = self.eval(f, obj)?;
                let i = self.eval(f, idx)?;
                match &o {
                    Value::List(l) => {
                        let len = l.borrow().len();
                        let k = self.index(&i, len, "القائمة")?;
                        l.borrow_mut().remove(k);
                        Ok(())
                    }
                    Value::Dict(d) => {
                        let key = self.key(&i)?;
                        if d.borrow_mut().remove(&key).is_none() {
                            return Err(self.key_error(&i));
                        }
                        Ok(())
                    }
                    _ => self.type_err(format!("لا يمكن حذف عناصر من {}", o.type_name())),
                }
            }
            Expr::Attr(obj, name) => {
                let o = self.eval(f, obj)?;
                if let Value::Instance(i) = &o {
                    if i.attrs.borrow_mut().remove(name).is_some() {
                        return Ok(());
                    }
                }
                Err(self.err("AttributeError", format!("{} لا يملك الخاصية '{name}'", o.type_name())))
            }
            Expr::Tuple(items) | Expr::List(items) => items.iter().try_for_each(|t| self.delete(f, t)),
            _ => self.type_err("لا يمكن حذف هذا التعبير"),
        }
    }

    // ---------- Expressions ----------

    pub fn eval(&mut self, f: &mut Frame, e: &Expr) -> R<Value> {
        Ok(match e {
            Expr::Const(v) => v.clone(),
            Expr::Name(n, var) => return self.load(f, n, *var),
            Expr::Bin(op, a, b) => {
                let a = self.eval(f, a)?;
                let b = self.eval(f, b)?;
                if let (Value::Int(x), Value::Int(y)) = (&a, &b) {
                    if let Some(r) = int_fast(*op, *x, *y) {
                        return Ok(Value::Int(r));
                    }
                }
                return self.binop(*op, a, b);
            }
            Expr::Compare(first, rest) if rest.len() == 1 => {
                let a = self.eval(f, first)?;
                let (op, right) = &rest[0];
                let b = self.eval(f, right)?;
                if let (Value::Int(x), Value::Int(y)) = (&a, &b) {
                    return Ok(Value::Bool(match op {
                        CmpOp::Eq => x == y,
                        CmpOp::Ne => x != y,
                        CmpOp::Lt => x < y,
                        CmpOp::Le => x <= y,
                        CmpOp::Gt => x > y,
                        CmpOp::Ge => x >= y,
                        _ => return Ok(Value::Bool(self.compare_op(*op, &a, &b)?)),
                    }));
                }
                Value::Bool(self.compare_op(*op, &a, &b)?)
            }
            Expr::Compare(first, rest) => {
                let mut left = self.eval(f, first)?;
                for (op, right) in rest {
                    let right = self.eval(f, right)?;
                    if !self.compare_op(*op, &left, &right)? {
                        return Ok(Value::Bool(false));
                    }
                    left = right;
                }
                Value::Bool(true)
            }
            Expr::Call(func, args) => return self.eval_call(f, func, args),
            Expr::Attr(obj, name) => {
                let o = self.eval(f, obj)?;
                return self.getattr(&o, name);
            }
            Expr::Index(obj, idx) => {
                let o = self.eval(f, obj)?;
                if let Expr::Slice(a, b, c) = &**idx {
                    let (a, b, c) = self.slice_parts(f, a, b, c)?;
                    return self.get_slice(&o, a, b, c);
                }
                let i = self.eval(f, idx)?;
                return self.getitem(&o, &i);
            }
            Expr::And(a, b) => {
                let a = self.eval(f, a)?;
                if !a.truthy() {
                    return Ok(a);
                }
                return self.eval(f, b);
            }
            Expr::Or(a, b) => {
                let a = self.eval(f, a)?;
                if a.truthy() {
                    return Ok(a);
                }
                return self.eval(f, b);
            }
            Expr::Unary(op, a) => {
                let v = self.eval(f, a)?;
                match (op, &v) {
                    (UnOp::Not, _) => Value::Bool(!v.truthy()),
                    (UnOp::Neg, Value::Int(i)) => Value::Int(i.checked_neg().ok_or_else(|| self.overflow())?),
                    (UnOp::Neg, Value::Bool(b)) => Value::Int(-(*b as i64)),
                    (UnOp::Neg, Value::Float(x)) => Value::Float(-x),
                    (UnOp::Pos, Value::Int(_) | Value::Float(_)) => v,
                    (UnOp::Pos, Value::Bool(b)) => Value::Int(*b as i64),
                    (UnOp::Invert, Value::Int(i)) => Value::Int(!i),
                    (UnOp::Invert, Value::Bool(b)) => Value::Int(!(*b as i64)),
                    _ => {
                        let sym = match op { UnOp::Neg => "-", UnOp::Pos => "+", _ => "~" };
                        return self.type_err(format!("لا يمكن تطبيق '{sym}' على {}", v.type_name()));
                    }
                }
            }
            Expr::IfExp(body, cond, orelse) => {
                return if self.eval(f, cond)?.truthy() { self.eval(f, body) } else { self.eval(f, orelse) };
            }
            _ => return self.eval_cold(f, e),
        })
    }

    /// Less common expressions, kept out of `eval` so its stack frame stays small
    #[inline(never)]
    fn eval_cold(&mut self, f: &mut Frame, e: &Expr) -> R<Value> {
        Ok(match e {
            Expr::FString(parts) => {
                let mut s = String::new();
                for p in parts {
                    match p {
                        FPart::Lit(t) => s.push_str(t),
                        FPart::Expr { expr, conv, spec } => {
                            let v = self.eval(f, expr)?;
                            let v = match conv {
                                Some('r') | Some('a') => Value::Str(self.repr(&v)?.into()),
                                Some('s') => Value::Str(self.to_str(&v)?.into()),
                                _ => v,
                            };
                            s.push_str(&builtins::format_value(self, &v, spec)?);
                        }
                    }
                }
                Value::Str(s.into())
            }
            Expr::List(items) => {
                let mut v = Vec::with_capacity(items.len());
                for i in items {
                    v.push(self.eval(f, i)?);
                }
                Value::list(v)
            }
            Expr::Tuple(items) => {
                let mut v = Vec::with_capacity(items.len());
                for i in items {
                    v.push(self.eval(f, i)?);
                }
                Value::tuple(v)
            }
            Expr::Dict(items) => {
                let mut d = Dict::default();
                for (k, v) in items {
                    let k = self.eval(f, k)?;
                    let v = self.eval(f, v)?;
                    let key = self.key(&k)?;
                    d.insert(key, k, v);
                }
                Value::Dict(Rc::new(RefCell::new(d)))
            }
            Expr::ListComp(elt, comps) => {
                let mut out = Vec::new();
                self.comprehension(f, comps, &mut |it, f| {
                    out.push(it.eval(f, elt)?);
                    Ok(())
                })?;
                Value::list(out)
            }
            Expr::DictComp(kv, comps) => {
                let mut d = Dict::default();
                self.comprehension(f, comps, &mut |it, f| {
                    let k = it.eval(f, &kv.0)?;
                    let v = it.eval(f, &kv.1)?;
                    let key = it.key(&k)?;
                    d.insert(key, k, v);
                    Ok(())
                })?;
                Value::Dict(Rc::new(RefCell::new(d)))
            }
            Expr::Slice(..) => return self.type_err("الشريحة مسموحة داخل [] فقط"),
            Expr::Lambda(def) => return self.make_function(f, def),
            _ => unreachable!("handled in eval"),
        })
    }

    fn comprehension(&mut self, f: &mut Frame, comps: &[Comp], sink: &mut dyn FnMut(&mut Interp, &mut Frame) -> R<()>) -> R<()> {
        let Some((c, rest)) = comps.split_first() else {
            return sink(self, f);
        };
        let it = self.eval(f, &c.iter)?;
        let items = match &it {
            Value::Range(r) => {
                let (start, _, st) = **r;
                (0..range_len(r)).map(|k| Value::Int(start + k * st)).collect()
            }
            _ => self.iterate(&it)?,
        };
        'items: for item in items {
            self.assign(f, &c.target, item)?;
            for cond in &c.conds {
                if !self.eval(f, cond)?.truthy() {
                    continue 'items;
                }
            }
            self.comprehension(f, rest, sink)?;
        }
        Ok(())
    }

    fn slice_parts(&mut self, f: &mut Frame, a: &Option<Box<Expr>>, b: &Option<Box<Expr>>, c: &Option<Box<Expr>>) -> R<(Option<i64>, Option<i64>, Option<i64>)> {
        let mut part = |e: &Option<Box<Expr>>| -> R<Option<i64>> {
            match e {
                None => Ok(None),
                Some(e) => match self.eval(f, e)? {
                    Value::None => Ok(None),
                    Value::Int(i) => Ok(Some(i)),
                    Value::Bool(b) => Ok(Some(b as i64)),
                    v => self.type_err(format!("حدود الشريحة يجب أن تكون أعدادا صحيحة، لا {}", v.type_name())),
                },
            }
        };
        Ok((part(a)?, part(b)?, part(c)?))
    }

    pub fn make_function(&mut self, f: &mut Frame, def: &Rc<FuncDef>) -> R<Value> {
        let mut defaults = Vec::with_capacity(def.params.len());
        for p in &def.params {
            defaults.push(match &p.default {
                Some(d) => self.eval(f, d)?,
                None => Value::Undef,
            });
        }
        let closure = def.freevars.iter().map(|v| match v {
            Var::Cell(i) => f.cells[*i as usize].clone(),
            Var::Free(i) => f.free[*i as usize].clone(),
            _ => unreachable!(),
        }).collect();
        Ok(Value::Func(Rc::new(Function {
            def: def.clone(),
            defaults,
            closure: Rc::new(closure),
            owner: RefCell::new(None),
        })))
    }

    // ---------- Calls ----------

    #[inline]
    fn eval_call(&mut self, f: &mut Frame, func: &Expr, args: &[Arg]) -> R<Value> {
        // obj.method(...) without creating a bound method
        if let Expr::Attr(obj, name) = func {
            let o = self.eval(f, obj)?;
            let (pos, kw) = self.eval_args(f, args, args.len() + 1)?;
            return self.call_method(&o, name, pos, kw);
        }
        let callee = self.eval(f, func)?;
        if let Value::Builtin(crate::value::Builtin::Super) = callee {
            if args.is_empty() {
                return self.zero_arg_super(f);
            }
        }
        let cap = match &callee {
            Value::Func(func) => func.def.nlocals as usize,
            Value::Class(_) => args.len() + 1, // room for self
            _ => 0,
        };
        let (pos, kw) = self.eval_args(f, args, cap)?;
        if let Value::Func(func) = &callee {
            return self.call_function(func, pos, kw);
        }
        self.call(&callee, pos, kw)
    }

    fn eval_args(&mut self, f: &mut Frame, args: &[Arg], cap: usize) -> R<(Vec<Value>, Vec<(Str, Value)>)> {
        let mut pos = self.vec_pool.pop().unwrap_or_default();
        pos.reserve(args.len().max(cap));
        let mut kw = Vec::new();
        for a in args {
            match a {
                Arg::Pos(e) => pos.push(self.eval(f, e)?),
                Arg::Star(e) => {
                    let v = self.eval(f, e)?;
                    pos.extend(self.iterate(&v)?);
                }
                Arg::Kw(n, e) => kw.push((n.clone(), self.eval(f, e)?)),
                Arg::StarStar(e) => match self.eval(f, e)? {
                    Value::Dict(d) => {
                        for (k, v) in d.borrow().iter() {
                            match k {
                                Value::Str(s) => kw.push((s.clone(), v.clone())),
                                _ => return self.type_err("مفاتيح ** يجب أن تكون نصوصا"),
                            }
                        }
                    }
                    v => return self.type_err(format!("بعد ** يجب أن يأتي قاموس، لا {}", v.type_name())),
                },
            }
        }
        Ok((pos, kw))
    }

    fn zero_arg_super(&mut self, f: &mut Frame) -> R<Value> {
        let func = f.func.clone();
        let owner = func.as_ref().and_then(|func| func.owner.borrow().clone());
        let (Some(func), Some(owner)) = (func, owner) else {
            return Err(self.err("RuntimeError", "super() تُستخدم داخل دوال الصنف فقط"));
        };
        let Some(first) = func.def.params.first() else {
            return Err(self.err("RuntimeError", "super() تحتاج معامل self"));
        };
        let this = self.load(f, &first.name, first.var)?;
        Ok(Value::Super(Rc::new((owner, this))))
    }

    pub fn call(&mut self, callee: &Value, mut args: Vec<Value>, kw: Vec<(Str, Value)>) -> R<Value> {
        match callee {
            Value::Func(func) => self.call_function(func, args, kw),
            Value::Builtin(b) => builtins::call_builtin(self, *b, args, kw),
            Value::Method(m) => {
                args.insert(0, m.0.clone());
                self.call(&m.1, args, kw)
            }
            Value::BuiltinMethod(m) => builtins::call_method(self, &m.0, &m.1, args, kw),
            Value::Class(c) => self.instantiate(c, args, kw),
            _ => self.type_err(format!("{} غير قابل للاستدعاء", callee.type_name())),
        }
    }

    pub fn call_method(&mut self, obj: &Value, name: &Str, mut args: Vec<Value>, kw: Vec<(Str, Value)>) -> R<Value> {
        match obj {
            Value::Instance(inst) => {
                if let Some(v) = inst.attrs.borrow().get(name).cloned() {
                    return self.call(&v, args, kw);
                }
                match inst.class.lookup(name) {
                    Some(Value::Func(func)) => {
                        args.insert(0, obj.clone());
                        self.call_function(&func, args, kw)
                    }
                    Some(v) => self.call(&v, args, kw),
                    None => Err(self.no_attr(obj, name)),
                }
            }
            Value::Super(s) => {
                let found = s.0.base.as_ref().and_then(|b| b.lookup(name));
                match found {
                    Some(Value::Func(func)) => {
                        args.insert(0, s.1.clone());
                        self.call_function(&func, args, kw)
                    }
                    Some(v) => self.call(&v, args, kw),
                    // Built-in exception classes have no __init__ of their own
                    None if &**name == "__init__" => {
                        if let Value::Instance(i) = &s.1 {
                            i.attrs.borrow_mut().insert(crate::lexer::intern("args"), Value::tuple(args));
                        }
                        Ok(Value::None)
                    }
                    None => Err(self.no_attr(obj, name)),
                }
            }
            Value::Class(_) | Value::Module(_) => {
                let v = self.getattr(obj, name)?;
                self.call(&v, args, kw)
            }
            Value::List(l) if &**name == "append" && args.len() == 1 && kw.is_empty() => {
                l.borrow_mut().push(args.pop().unwrap());
                Ok(Value::None)
            }
            _ => builtins::call_method(self, obj, name, args, kw),
        }
    }

    #[inline]
    pub fn call_function(&mut self, func: &Rc<Function>, args: Vec<Value>, kw: Vec<(Str, Value)>) -> R<Value> {
        let def = &func.def;
        // Fast path: exactly the positional arguments, which go straight into the parameter slots
        if def.simple_params && kw.is_empty() && args.len() == def.params.len() && self.depth < MAX_DEPTH {
            let mut locals = args;
            locals.resize(def.nlocals as usize, Value::Undef);
            let mut frame = Frame {
                locals,
                cells: self.new_cells(def.ncells),
                free: func.closure.clone(),
                func: Some(func.clone()),
                class_ns: None,
            };
            let r = self.run_body(func, &mut frame);
            if self.vec_pool.len() < 256 {
                let mut v = std::mem::take(&mut frame.locals);
                v.clear();
                self.vec_pool.push(v);
            }
            return r;
        }
        self.call_function_general(func, args, kw)
    }

    fn new_cells(&self, n: u32) -> Vec<Rc<RefCell<Value>>> {
        (0..n).map(|_| Rc::new(RefCell::new(Value::Undef))).collect()
    }

    /// Defaults, keyword arguments, *args and **kwargs
    #[inline(never)]
    fn call_function_general(&mut self, func: &Rc<Function>, args: Vec<Value>, kw: Vec<(Str, Value)>) -> R<Value> {
        let def = &func.def;
        if self.depth >= MAX_DEPTH {
            return Err(self.err("RecursionError", "تجاوز الحد الأقصى لعمق الاستدعاء (استدعاء ذاتي بلا نهاية؟)"));
        }
        let np = def.params.len();
        let mut vals: Vec<Value> = vec![Value::Undef; np];
        let mut extra = Vec::new();
        for (i, a) in args.into_iter().enumerate() {
            if i < np {
                vals[i] = a;
            } else if def.vararg.is_some() {
                extra.push(a);
            } else {
                return self.type_err(format!("الدالة {}() تأخذ {} وسائط فقط", def.name, np));
            }
        }
        let mut kwdict = Dict::default();
        for (k, v) in kw {
            match def.params.iter().position(|p| p.name == k) {
                Some(i) => {
                    if !matches!(vals[i], Value::Undef) {
                        return self.type_err(format!("الدالة {}() أُعطيت أكثر من قيمة للمعامل '{k}'", def.name));
                    }
                    vals[i] = v;
                }
                None if def.kwarg.is_some() => kwdict.insert(Key::Str(k.clone()), Value::Str(k), v),
                None => return self.type_err(format!("الدالة {}() لا تملك معاملا باسم '{k}'", def.name)),
            }
        }
        let mut frame = Frame {
            locals: vec![Value::Undef; def.nlocals as usize],
            cells: self.new_cells(def.ncells),
            free: func.closure.clone(),
            func: Some(func.clone()),
            class_ns: None,
        };
        for (i, (p, v)) in def.params.iter().zip(vals).enumerate() {
            let v = match v {
                Value::Undef => match &func.defaults[i] {
                    Value::Undef => return self.type_err(format!("الدالة {}() ينقصها المعامل '{}'", def.name, p.name)),
                    d => d.clone(),
                },
                v => v,
            };
            self.store(&mut frame, &p.name, p.var, v);
        }
        if let Some((n, var)) = &def.vararg {
            self.store(&mut frame, n, *var, Value::tuple(extra));
        }
        if let Some((n, var)) = &def.kwarg {
            self.store(&mut frame, n, *var, Value::Dict(Rc::new(RefCell::new(kwdict))));
        }
        self.run_body(func, &mut frame)
    }

    #[inline]
    fn run_body(&mut self, func: &Rc<Function>, frame: &mut Frame) -> R<Value> {
        self.depth += 1;
        let r = self.exec_block(frame, &func.def.body);
        self.depth -= 1;
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(Value::None),
        }
    }

    fn instantiate(&mut self, cls: &Rc<Class>, mut args: Vec<Value>, kw: Vec<(Str, Value)>) -> R<Value> {
        let inst = Rc::new(Instance { class: cls.clone(), attrs: RefCell::new(AttrMap::default()) });
        let v = Value::Instance(inst.clone());
        let is_exc = self.is_exception_class(cls);
        if is_exc {
            inst.attrs.borrow_mut().insert(crate::lexer::intern("args"), Value::tuple(args.clone()));
        }
        let init_name = self.init_name.clone();
        match cls.lookup(&init_name) {
            Some(Value::Func(init)) => {
                args.insert(0, v.clone());
                self.call_function(&init, args, kw)?;
            }
            Some(init) => {
                args.insert(0, v.clone());
                self.call(&init, args, kw)?;
            }
            None if !is_exc && (!args.is_empty() || !kw.is_empty()) => {
                return self.type_err(format!("الصنف {} لا يأخذ وسائط (عرّف الدالة __init__)", cls.name));
            }
            None => {}
        }
        Ok(v)
    }

    // ---------- Attributes ----------

    pub fn no_attr(&self, obj: &Value, name: &str) -> Box<Exc> {
        self.err("AttributeError", format!("{} لا يملك الخاصية '{name}'", obj.type_name()))
    }

    pub fn getattr(&mut self, obj: &Value, name: &Str) -> R<Value> {
        match obj {
            Value::Instance(inst) => {
                if let Some(v) = inst.attrs.borrow().get(name) {
                    return Ok(v.clone());
                }
                if &**name == "__class__" {
                    return Ok(Value::Class(inst.class.clone()));
                }
                match inst.class.lookup(name) {
                    Some(f @ Value::Func(_)) => Ok(Value::Method(Rc::new((obj.clone(), f)))),
                    Some(v) => Ok(v),
                    None => Err(self.no_attr(obj, name)),
                }
            }
            Value::Class(c) => {
                if &**name == "__name__" {
                    return Ok(Value::Str(c.name.clone()));
                }
                c.lookup(name).ok_or_else(|| self.no_attr(obj, name))
            }
            Value::Module(m) => m.attrs.borrow().get(name).cloned().ok_or_else(|| {
                self.err("AttributeError", format!("الوحدة '{}' لا تحتوي '{name}'", m.name))
            }),
            Value::Super(s) => match s.0.base.as_ref().and_then(|b| b.lookup(name)) {
                Some(f @ Value::Func(_)) => Ok(Value::Method(Rc::new((s.1.clone(), f)))),
                Some(v) => Ok(v),
                None => Err(self.no_attr(obj, name)),
            },
            Value::Func(func) if &**name == "__name__" => Ok(Value::Str(func.def.name.clone())),
            Value::Builtin(b) if &**name == "__name__" => Ok(Value::str(b.name())),
            _ if builtins::has_method(obj, name) => Ok(Value::BuiltinMethod(Rc::new((obj.clone(), name.clone())))),
            _ => Err(self.no_attr(obj, name)),
        }
    }

    pub fn setattr(&mut self, obj: &Value, name: &Str, v: Value) -> R<()> {
        match obj {
            Value::Instance(inst) => {
                inst.attrs.borrow_mut().insert(name.clone(), v);
                Ok(())
            }
            Value::Class(c) => {
                c.attrs.borrow_mut().insert(name.clone(), v);
                Ok(())
            }
            _ => Err(self.err("AttributeError", format!("لا يمكن إضافة خصائص إلى {}", obj.type_name()))),
        }
    }

    /// Calls a dunder method like __str__ if the instance's class defines one
    fn dunder(&mut self, obj: &Value, name: &str, args: Vec<Value>) -> R<Option<Value>> {
        if let Value::Instance(inst) = obj {
            if let Some(Value::Func(f)) = inst.class.lookup(name) {
                let mut a = Vec::with_capacity(args.len() + 1);
                a.push(obj.clone());
                a.extend(args);
                return self.call_function(&f, a, Vec::new()).map(Some);
            }
        }
        Ok(None)
    }

    fn import(&mut self, name: &Str) -> R<Value> {
        if let Some(m) = self.modules.get(name) {
            return Ok(m.clone());
        }
        let Some(attrs) = builtins::module(name) else {
            return Err(self.err("ModuleNotFoundError", format!("لا توجد وحدة باسم '{name}' (المتاح: math، random، time)")));
        };
        let m = Value::Module(Rc::new(Module { name: name.clone(), attrs: RefCell::new(attrs) }));
        self.modules.insert(name.clone(), m.clone());
        Ok(m)
    }

    // ---------- Operators ----------

    pub fn overflow(&self) -> Box<Exc> {
        self.err("OverflowError", "الناتج أكبر من أن يُخزّن في عدد صحيح (64 بت)")
    }

    fn zero_div(&self) -> Box<Exc> {
        self.err("ZeroDivisionError", "لا يمكن القسمة على صفر")
    }

    pub fn binop(&mut self, op: BinOp, a: Value, b: Value) -> R<Value> {
        use Value::*;
        // Fast path: both ints
        if let (Int(x), Int(y)) = (&a, &b) {
            return self.int_op(op, *x, *y);
        }
        if is_num(&a) && is_num(&b) {
            return match (num(&a), num(&b)) {
                (Num::I(x), Num::I(y)) => self.int_op(op, x, y),
                (x, y) => self.float_op(op, x.f(), y.f()),
            };
        }
        let result = match (op, &a, &b) {
            (BinOp::Add, Str(x), Str(y)) => {
                let mut s = String::with_capacity(x.len() + y.len());
                s.push_str(x);
                s.push_str(y);
                Some(Str(s.into()))
            }
            (BinOp::Mul, Str(s), Int(n)) | (BinOp::Mul, Int(n), Str(s)) => Some(Str(s.repeat((*n).max(0) as usize).into())),
            (BinOp::Add, List(x), List(y)) => {
                let mut v = x.borrow().clone();
                v.extend(y.borrow().iter().cloned());
                Some(Value::list(v))
            }
            (BinOp::Mul, List(l), Int(n)) | (BinOp::Mul, Int(n), List(l)) => {
                let l = l.borrow();
                let mut v = Vec::with_capacity(l.len() * (*n).max(0) as usize);
                for _ in 0..(*n).max(0) {
                    v.extend(l.iter().cloned());
                }
                Some(Value::list(v))
            }
            (BinOp::Add, Tuple(x), Tuple(y)) => Some(Value::tuple(x.iter().chain(y.iter()).cloned().collect())),
            (BinOp::Mul, Tuple(t), Int(n)) | (BinOp::Mul, Int(n), Tuple(t)) => {
                Some(Value::tuple((0..(*n).max(0)).flat_map(|_| t.iter().cloned()).collect()))
            }
            (BinOp::Mod, Str(fmt), _) => Some(Str(builtins::percent_format(self, fmt, &b)?.into())),
            _ => Option::None,
        };
        if let Some(r) = result {
            return Ok(r);
        }
        let (dunder, sym) = match op {
            BinOp::Add => ("__add__", "+"),
            BinOp::Sub => ("__sub__", "-"),
            BinOp::Mul => ("__mul__", "*"),
            BinOp::Div => ("__truediv__", "/"),
            BinOp::FloorDiv => ("__floordiv__", "//"),
            BinOp::Mod => ("__mod__", "%"),
            BinOp::Pow => ("__pow__", "**"),
            BinOp::BitAnd => ("__and__", "&"),
            BinOp::BitOr => ("__or__", "|"),
            BinOp::BitXor => ("__xor__", "^"),
            BinOp::Shl => ("__lshift__", "<<"),
            BinOp::Shr => ("__rshift__", ">>"),
        };
        if let Some(r) = self.dunder(&a, dunder, vec![b.clone()])? {
            return Ok(r);
        }
        let hint = if matches!((&a, &b), (Str(_), Int(_) | Float(_)) | (Int(_) | Float(_), Str(_))) && op == BinOp::Add {
            " (حوّل الرقم إلى نص بـ نص() أو النص إلى رقم بـ عدد_صحيح())"
        } else {
            ""
        };
        self.type_err(format!("لا يمكن تطبيق '{sym}' على {} و{}{hint}", a.type_name(), b.type_name()))
    }

    fn int_op(&self, op: BinOp, x: i64, y: i64) -> R<Value> {
        let ov = || self.overflow();
        Ok(Value::Int(match op {
            BinOp::Add => x.checked_add(y).ok_or_else(ov)?,
            BinOp::Sub => x.checked_sub(y).ok_or_else(ov)?,
            BinOp::Mul => x.checked_mul(y).ok_or_else(ov)?,
            BinOp::Div => {
                if y == 0 {
                    return Err(self.zero_div());
                }
                return Ok(Value::Float(x as f64 / y as f64));
            }
            BinOp::FloorDiv => {
                if y == 0 {
                    return Err(self.zero_div());
                }
                let q = x.checked_div(y).ok_or_else(ov)?;
                if x % y != 0 && ((x < 0) != (y < 0)) { q - 1 } else { q }
            }
            BinOp::Mod => {
                if y == 0 {
                    return Err(self.zero_div());
                }
                let r = x.checked_rem(y).unwrap_or(0);
                if r != 0 && ((r < 0) != (y < 0)) { r + y } else { r }
            }
            BinOp::Pow => {
                if y < 0 {
                    if x == 0 {
                        return Err(self.zero_div());
                    }
                    return Ok(Value::Float((x as f64).powf(y as f64)));
                }
                let e = u32::try_from(y).map_err(|_| ov())?;
                x.checked_pow(e).ok_or_else(ov)?
            }
            BinOp::BitAnd => x & y,
            BinOp::BitOr => x | y,
            BinOp::BitXor => x ^ y,
            BinOp::Shl => {
                if y < 0 {
                    return Err(self.err("ValueError", "عدد الإزاحة سالب"));
                }
                if y >= 64 || (x << y) >> y != x {
                    return Err(ov());
                }
                x << y
            }
            BinOp::Shr => {
                if y < 0 {
                    return Err(self.err("ValueError", "عدد الإزاحة سالب"));
                }
                x >> y.min(63)
            }
        }))
    }

    fn float_op(&self, op: BinOp, x: f64, y: f64) -> R<Value> {
        Ok(Value::Float(match op {
            BinOp::Add => x + y,
            BinOp::Sub => x - y,
            BinOp::Mul => x * y,
            BinOp::Div | BinOp::FloorDiv | BinOp::Mod if y == 0.0 => return Err(self.zero_div()),
            BinOp::Div => x / y,
            BinOp::FloorDiv => (x / y).floor(),
            BinOp::Mod => {
                let r = x % y;
                if r != 0.0 && ((r < 0.0) != (y < 0.0)) { r + y } else { r }
            }
            BinOp::Pow => {
                if x == 0.0 && y < 0.0 {
                    return Err(self.zero_div());
                }
                if x < 0.0 && y.fract() != 0.0 {
                    return Err(self.err("ValueError", "لا يمكن رفع عدد سالب إلى أس كسري"));
                }
                x.powf(y)
            }
            _ => return self.type_err("العمليات الثنائية (& | ^ << >>) تعمل على الأعداد الصحيحة فقط"),
        }))
    }

    pub fn eq(&mut self, a: &Value, b: &Value) -> R<bool> {
        if let Value::Instance(_) = a {
            if let Some(r) = self.dunder(a, "__eq__", vec![b.clone()])? {
                return Ok(r.truthy());
            }
        }
        Ok(values_eq(a, b))
    }

    pub fn compare_op(&mut self, op: CmpOp, a: &Value, b: &Value) -> R<bool> {
        use std::cmp::Ordering::*;
        Ok(match op {
            CmpOp::Eq => self.eq(a, b)?,
            CmpOp::Ne => !self.eq(a, b)?,
            CmpOp::In => self.contains(b, a)?,
            CmpOp::NotIn => !self.contains(b, a)?,
            CmpOp::Is | CmpOp::IsNot => {
                let same = match (a, b) {
                    (Value::None, Value::None) => true,
                    (Value::Bool(x), Value::Bool(y)) => x == y,
                    (Value::Int(x), Value::Int(y)) => x == y,
                    (Value::Str(x), Value::Str(y)) => Rc::ptr_eq(x, y) || x == y,
                    (Value::List(x), Value::List(y)) => Rc::ptr_eq(x, y),
                    (Value::Tuple(x), Value::Tuple(y)) => Rc::ptr_eq(x, y),
                    (Value::Dict(x), Value::Dict(y)) => Rc::ptr_eq(x, y),
                    _ => values_eq(a, b) && !matches!(a, Value::Float(_)),
                };
                same == (op == CmpOp::Is)
            }
            _ => {
                let ord = match compare(a, b) {
                    Some(o) => o,
                    None => {
                        let (name, sym) = match op {
                            CmpOp::Lt => ("__lt__", "<"),
                            CmpOp::Le => ("__le__", "<="),
                            CmpOp::Gt => ("__gt__", ">"),
                            _ => ("__ge__", ">="),
                        };
                        if let Some(r) = self.dunder(a, name, vec![b.clone()])? {
                            return Ok(r.truthy());
                        }
                        if is_num(a) && is_num(b) {
                            return Ok(false); // NaN
                        }
                        return self.type_err(format!("لا يمكن مقارنة {} و{} بـ '{sym}'", a.type_name(), b.type_name()));
                    }
                };
                match op {
                    CmpOp::Lt => ord == Less,
                    CmpOp::Le => ord != Greater,
                    CmpOp::Gt => ord == Greater,
                    _ => ord != Less,
                }
            }
        })
    }

    pub fn contains(&mut self, container: &Value, item: &Value) -> R<bool> {
        match container {
            Value::Str(s) => match item {
                Value::Str(sub) => Ok(s.contains(&**sub)),
                _ => self.type_err(format!("لا يمكن البحث عن {} داخل نص", item.type_name())),
            },
            Value::List(l) => {
                let items = l.borrow().clone();
                for v in &items {
                    if self.eq(v, item)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Value::Tuple(t) => {
                for v in t.iter() {
                    if self.eq(v, item)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Value::Dict(d) => Ok(Key::of(item).is_some_and(|k| d.borrow().get(&k).is_some())),
            Value::Range(r) => Ok(match item {
                Value::Int(i) => {
                    let (start, stop, st) = **r;
                    let inside = if st > 0 { *i >= start && *i < stop } else { *i <= start && *i > stop };
                    inside && (i - start) % st == 0
                }
                _ => false,
            }),
            Value::Instance(_) => match self.dunder(container, "__contains__", vec![item.clone()])? {
                Some(r) => Ok(r.truthy()),
                None => self.type_err(format!("{} لا يدعم 'في'", container.type_name())),
            },
            _ => self.type_err(format!("{} لا يدعم 'في'", container.type_name())),
        }
    }

    // ---------- Indexing ----------

    pub fn key(&self, v: &Value) -> R<Key> {
        Key::of(v).ok_or_else(|| self.err("TypeError", format!("{} لا يصلح مفتاحا في قاموس (قابل للتغيير)", v.type_name())))
    }

    pub fn key_error(&mut self, k: &Value) -> Box<Exc> {
        let r = self.repr(k).unwrap_or_default();
        self.err("KeyError", r)
    }

    /// Normalizes a (possibly negative) index into 0..len
    pub fn index(&self, i: &Value, len: usize, what: &str) -> R<usize> {
        let i = match i {
            Value::Int(i) => *i,
            Value::Bool(b) => *b as i64,
            v => return self.type_err(format!("فهرس {what} يجب أن يكون عددا صحيحا، لا {}", v.type_name())),
        };
        let k = if i < 0 { i + len as i64 } else { i };
        if k < 0 || k >= len as i64 {
            return Err(self.err("IndexError", format!("الفهرس {i} خارج نطاق {what} (الطول {len})")));
        }
        Ok(k as usize)
    }

    pub fn getitem(&mut self, obj: &Value, i: &Value) -> R<Value> {
        match obj {
            Value::List(l) => {
                let l = l.borrow();
                let k = self.index(i, l.len(), "القائمة")?;
                Ok(l[k].clone())
            }
            Value::Tuple(t) => {
                let k = self.index(i, t.len(), "الصف")?;
                Ok(t[k].clone())
            }
            Value::Str(s) => {
                if s.is_ascii() {
                    let k = self.index(i, s.len(), "النص")?;
                    return Ok(Value::Str(s[k..k + 1].into()));
                }
                let chars: Vec<char> = s.chars().collect();
                let k = self.index(i, chars.len(), "النص")?;
                Ok(Value::Str(chars[k].to_string().into()))
            }
            Value::Dict(d) => {
                let key = self.key(i)?;
                let found = d.borrow().get(&key).cloned();
                found.ok_or_else(|| self.key_error(i))
            }
            Value::Range(r) => {
                let k = self.index(i, range_len(r) as usize, "النطاق")?;
                Ok(Value::Int(r.0 + k as i64 * r.2))
            }
            Value::Instance(_) => match self.dunder(obj, "__getitem__", vec![i.clone()])? {
                Some(v) => Ok(v),
                None => self.type_err(format!("{} لا يدعم الفهرسة []", obj.type_name())),
            },
            _ => self.type_err(format!("{} لا يدعم الفهرسة []", obj.type_name())),
        }
    }

    pub fn setitem(&mut self, obj: &Value, i: Value, v: Value) -> R<()> {
        match obj {
            Value::List(l) => {
                let len = l.borrow().len();
                let k = self.index(&i, len, "القائمة")?;
                l.borrow_mut()[k] = v;
                Ok(())
            }
            Value::Dict(d) => {
                let key = self.key(&i)?;
                d.borrow_mut().insert(key, i, v);
                Ok(())
            }
            Value::Instance(_) => match self.dunder(obj, "__setitem__", vec![i, v])? {
                Some(_) => Ok(()),
                None => self.type_err(format!("{} لا يدعم الإسناد بالفهرس", obj.type_name())),
            },
            Value::Str(_) => self.type_err("النصوص لا تتغير؛ أنشئ نصا جديدا بدلا من ذلك"),
            Value::Tuple(_) => self.type_err("الصفوف لا تتغير؛ استخدم قائمة بدلا من ذلك"),
            _ => self.type_err(format!("{} لا يدعم الإسناد بالفهرس", obj.type_name())),
        }
    }

    /// Indices selected by a slice over a sequence of length `len`
    fn slice_indices(&self, len: usize, a: Option<i64>, b: Option<i64>, c: Option<i64>) -> R<Vec<usize>> {
        let len = len as i64;
        let step = c.unwrap_or(1);
        if step == 0 {
            return Err(self.err("ValueError", "خطوة الشريحة لا يمكن أن تكون صفرا"));
        }
        let clamp = |x: i64, lo: i64, hi: i64| x.max(lo).min(hi);
        let norm = |x: i64| if x < 0 { x + len } else { x };
        let mut out = Vec::new();
        if step > 0 {
            let start = a.map_or(0, |x| clamp(norm(x), 0, len));
            let stop = b.map_or(len, |x| clamp(norm(x), 0, len));
            let mut i = start;
            while i < stop {
                out.push(i as usize);
                i += step;
            }
        } else {
            let start = a.map_or(len - 1, |x| clamp(norm(x), -1, len - 1));
            let stop = b.map_or(-1, |x| clamp(norm(x), -1, len - 1));
            let mut i = start;
            while i > stop {
                out.push(i as usize);
                i += step;
            }
        }
        Ok(out)
    }

    fn get_slice(&mut self, obj: &Value, a: Option<i64>, b: Option<i64>, c: Option<i64>) -> R<Value> {
        match obj {
            Value::List(l) => {
                let l = l.borrow();
                let idx = self.slice_indices(l.len(), a, b, c)?;
                Ok(Value::list(idx.into_iter().map(|i| l[i].clone()).collect()))
            }
            Value::Tuple(t) => {
                let idx = self.slice_indices(t.len(), a, b, c)?;
                Ok(Value::tuple(idx.into_iter().map(|i| t[i].clone()).collect()))
            }
            Value::Str(s) => {
                let chars: Vec<char> = s.chars().collect();
                let idx = self.slice_indices(chars.len(), a, b, c)?;
                Ok(Value::Str(idx.into_iter().map(|i| chars[i]).collect::<String>().into()))
            }
            Value::Range(r) => {
                let items: Vec<Value> = (0..range_len(r)).map(|k| Value::Int(r.0 + k * r.2)).collect();
                let idx = self.slice_indices(items.len(), a, b, c)?;
                Ok(Value::list(idx.into_iter().map(|i| items[i].clone()).collect()))
            }
            _ => self.type_err(format!("{} لا يدعم الشرائح [:]", obj.type_name())),
        }
    }

    fn set_slice(&mut self, obj: &Value, a: Option<i64>, b: Option<i64>, c: Option<i64>, v: Value) -> R<()> {
        let Value::List(l) = obj else {
            return self.type_err(format!("{} لا يدعم الإسناد إلى شريحة", obj.type_name()));
        };
        let items = self.iterate(&v)?;
        let len = l.borrow().len();
        let idx = self.slice_indices(len, a, b, c)?;
        if c.unwrap_or(1) == 1 {
            let start = idx.first().copied().unwrap_or_else(|| {
                let s = a.unwrap_or(0);
                (if s < 0 { s + len as i64 } else { s }).clamp(0, len as i64) as usize
            });
            l.borrow_mut().splice(start..start + idx.len(), items);
            return Ok(());
        }
        if idx.len() != items.len() {
            return Err(self.err("ValueError", "عدد العناصر لا يساوي طول الشريحة"));
        }
        let mut l = l.borrow_mut();
        for (i, v) in idx.into_iter().zip(items) {
            l[i] = v;
        }
        Ok(())
    }

    /// All items of an iterable, collected
    pub fn iterate(&mut self, v: &Value) -> R<Vec<Value>> {
        Ok(match v {
            Value::List(l) => l.borrow().clone(),
            Value::Tuple(t) => t.to_vec(),
            Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string().into())).collect(),
            Value::Dict(d) => d.borrow().iter().map(|(k, _)| k.clone()).collect(),
            Value::Range(r) => {
                let n = range_len(r);
                if n > 100_000_000 {
                    return Err(self.err("OverflowError", "النطاق كبير جدا"));
                }
                (0..n).map(|k| Value::Int(r.0 + k * r.2)).collect()
            }
            _ => return self.type_err(format!("{} غير قابل للتكرار (لا يمكن المرور على عناصره)", v.type_name())),
        })
    }

    // ---------- Conversions to text ----------

    pub fn to_str(&mut self, v: &Value) -> R<String> {
        match v {
            Value::Str(s) => Ok(s.to_string()),
            Value::Instance(inst) => {
                if let Some(r) = self.dunder(v, "__str__", Vec::new())? {
                    return self.to_str(&r);
                }
                if self.is_exception_class(&inst.class) && inst.class.lookup("__repr__").is_none() {
                    let args = inst.attrs.borrow().get("args").cloned();
                    return match args {
                        Some(Value::Tuple(t)) if t.len() == 1 => self.to_str(&t[0]),
                        Some(Value::Tuple(t)) if t.is_empty() => Ok(String::new()),
                        Some(a) => self.repr(&a),
                        None => Ok(String::new()),
                    };
                }
                self.repr(v)
            }
            _ => self.repr(v),
        }
    }

    pub fn repr(&mut self, v: &Value) -> R<String> {
        let mut s = String::new();
        self.repr_into(v, &mut s, 0)?;
        Ok(s)
    }

    fn repr_into(&mut self, v: &Value, s: &mut String, depth: usize) -> R<()> {
        use std::fmt::Write;
        if depth > 100 {
            s.push_str("...");
            return Ok(());
        }
        match v {
            Value::Undef | Value::None => s.push_str("None"),
            Value::Bool(b) => s.push_str(if *b { "True" } else { "False" }),
            Value::Int(i) => {
                let _ = write!(s, "{i}");
            }
            Value::Float(f) => s.push_str(&float_repr(*f)),
            Value::Str(t) => s.push_str(&str_repr(t)),
            Value::List(l) => {
                let items = l.borrow().clone();
                s.push('[');
                for (i, x) in items.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    self.repr_into(x, s, depth + 1)?;
                }
                s.push(']');
            }
            Value::Tuple(t) => {
                s.push('(');
                for (i, x) in t.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    self.repr_into(x, s, depth + 1)?;
                }
                if t.len() == 1 {
                    s.push(',');
                }
                s.push(')');
            }
            Value::Dict(d) => {
                let items: Vec<(Value, Value)> = d.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                s.push('{');
                for (i, (k, x)) in items.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    self.repr_into(k, s, depth + 1)?;
                    s.push_str(": ");
                    self.repr_into(x, s, depth + 1)?;
                }
                s.push('}');
            }
            Value::Range(r) => {
                let _ = if r.2 == 1 { write!(s, "range({}, {})", r.0, r.1) } else { write!(s, "range({}, {}, {})", r.0, r.1, r.2) };
            }
            Value::Func(f) => {
                let _ = write!(s, "<function {}>", f.def.name);
            }
            Value::Builtin(b) if b.is_type() => {
                let _ = write!(s, "<class '{}'>", b.name());
            }
            Value::Builtin(b) => {
                let _ = write!(s, "<built-in function {}>", b.name());
            }
            Value::Method(m) => {
                let name = match &m.1 { Value::Func(f) => f.def.name.to_string(), _ => "?".into() };
                let _ = write!(s, "<bound method {name}>");
            }
            Value::BuiltinMethod(m) => {
                let _ = write!(s, "<built-in method {}>", m.1);
            }
            Value::Class(c) => {
                let _ = write!(s, "<class '{}'>", c.name);
            }
            Value::Instance(inst) => {
                if let Some(r) = self.dunder(v, "__repr__", Vec::new())? {
                    s.push_str(&self.to_str(&r)?);
                } else if self.is_exception_class(&inst.class) {
                    let args = inst.attrs.borrow().get("args").cloned().unwrap_or(Value::tuple(vec![]));
                    let a = self.repr(&args)?;
                    let a = a.strip_suffix(",)").map(|x| format!("{x})")).unwrap_or(a);
                    let _ = write!(s, "{}{}", inst.class.name, a);
                } else {
                    let _ = write!(s, "<{} object>", inst.class.name);
                }
            }
            Value::Module(m) => {
                let _ = write!(s, "<module '{}'>", m.name);
            }
            Value::Super(_) => s.push_str("<super>"),
        }
        Ok(())
    }

    // ---------- Input ----------

    pub fn read_line(&mut self, prompt: &str) -> R<String> {
        self.out.push_str(prompt);
        self.flush();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => Err(self.err("EOFError", "لا يوجد إدخال (انتهت المدخلات)")),
            Ok(_) => {
                while line.ends_with('\n') || line.ends_with('\r') {
                    line.pop();
                }
                Ok(line)
            }
        }
    }
}
