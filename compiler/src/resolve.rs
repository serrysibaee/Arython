// Resolver: decides where every variable lives, so the interpreter can use array
// slots instead of name lookups. Follows Python's rules: a name assigned anywhere
// in a function is local to it unless declared global/nonlocal.

use crate::ast::*;
use crate::lexer::SyntaxError;
use crate::value::Str;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Global variable names ↔ slots; shared across REPL inputs
#[derive(Default)]
pub struct Globals {
    pub names: Vec<Str>,
    pub index: HashMap<Str, u32>,
}

impl Globals {
    pub fn slot(&mut self, name: &Str) -> u32 {
        if let Some(&i) = self.index.get(name) {
            return i;
        }
        let i = self.names.len() as u32;
        self.names.push(name.clone());
        self.index.insert(name.clone(), i);
        i
    }
}

enum ScopeKind {
    Module,
    Function,
    Class,
}

struct Scope {
    kind: ScopeKind,
    locals: HashMap<Str, u32>,
    cells: HashMap<Str, u32>,
    frees: HashMap<Str, u32>,
    freevars: Vec<Var>,
    globals: HashSet<Str>,
    /// Class scopes: names assigned in the class body
    class_names: HashSet<Str>,
}

pub struct Resolver<'g> {
    globals: &'g mut Globals,
    scopes: Vec<Scope>,
}

type RResult = Result<(), SyntaxError>;

pub fn resolve_module(body: &mut [Stmt], globals: &mut Globals) -> RResult {
    let mut r = Resolver { globals, scopes: Vec::new() };
    r.scopes.push(Scope {
        kind: ScopeKind::Module, locals: HashMap::new(), cells: HashMap::new(), frees: HashMap::new(),
        freevars: Vec::new(), globals: HashSet::new(), class_names: HashSet::new(),
    });
    r.block(body)
}

// ---------- Name collection ----------

/// Names bound directly in a block (not inside nested functions), plus global/nonlocal declarations
fn bound_names(body: &[Stmt], out: &mut Vec<Str>, globals: &mut HashSet<Str>, nonlocals: &mut HashSet<Str>) {
    for s in body {
        match &s.kind {
            StmtKind::Assign(targets, value) => {
                targets.iter().for_each(|t| target_names(t, out));
                expr_bound(value, out);
            }
            StmtKind::AugAssign(t, _, v) => {
                target_names(t, out);
                expr_bound(v, out);
            }
            StmtKind::Expr(e) | StmtKind::Return(Some(e)) | StmtKind::Raise(Some(e)) | StmtKind::Assert(e, _) => expr_bound(e, out),
            StmtKind::For(t, iter, body, orelse) => {
                target_names(t, out);
                expr_bound(iter, out);
                bound_names(body, out, globals, nonlocals);
                bound_names(orelse, out, globals, nonlocals);
            }
            StmtKind::While(c, body, orelse) => {
                expr_bound(c, out);
                bound_names(body, out, globals, nonlocals);
                bound_names(orelse, out, globals, nonlocals);
            }
            StmtKind::If(branches, orelse) => {
                for (c, b) in branches {
                    expr_bound(c, out);
                    bound_names(b, out, globals, nonlocals);
                }
                bound_names(orelse, out, globals, nonlocals);
            }
            StmtKind::Try { body, handlers, orelse, finally } => {
                bound_names(body, out, globals, nonlocals);
                for h in handlers {
                    if let Some(n) = &h.name {
                        target_names(n, out);
                    }
                    bound_names(&h.body, out, globals, nonlocals);
                }
                bound_names(orelse, out, globals, nonlocals);
                bound_names(finally, out, globals, nonlocals);
            }
            StmtKind::Def(_, t) | StmtKind::Class { target: t, .. } => target_names(t, out),
            StmtKind::Del(ts) => ts.iter().for_each(|t| target_names(t, out)),
            StmtKind::Import(items) | StmtKind::ImportFrom(_, items) => items.iter().for_each(|(_, t)| target_names(t, out)),
            StmtKind::Global(ns) => globals.extend(ns.iter().cloned()),
            StmtKind::Nonlocal(ns) => nonlocals.extend(ns.iter().cloned()),
            _ => {}
        }
    }
}

fn target_names(t: &Expr, out: &mut Vec<Str>) {
    match t {
        Expr::Name(n, _) => out.push(n.clone()),
        Expr::Tuple(items) | Expr::List(items) => items.iter().for_each(|i| target_names(i, out)),
        _ => {}
    }
}

/// Comprehension loop variables are bound in the enclosing scope
fn expr_bound(e: &Expr, out: &mut Vec<Str>) {
    walk_expr(e, &mut |e| {
        if let Expr::ListComp(_, comps) | Expr::DictComp(_, comps) = e {
            comps.iter().for_each(|c| target_names(&c.target, out));
        }
    });
}

/// Calls `f` on every expression inside `e`, not descending into lambdas
fn walk_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    let mut w = |x: &Expr| walk_expr(x, f);
    match e {
        Expr::FString(parts) => parts.iter().for_each(|p| if let FPart::Expr { expr, .. } = p { w(expr) }),
        Expr::List(items) | Expr::Tuple(items) => items.iter().for_each(w),
        Expr::Dict(items) => items.iter().for_each(|(k, v)| { w(k); w(v) }),
        Expr::ListComp(elt, comps) => {
            w(elt);
            comps.iter().for_each(|c| { w(&c.iter); c.conds.iter().for_each(&mut w) });
        }
        Expr::DictComp(kv, comps) => {
            w(&kv.0);
            w(&kv.1);
            comps.iter().for_each(|c| { w(&c.iter); c.conds.iter().for_each(&mut w) });
        }
        Expr::Bin(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Index(a, b) => { w(a); w(b) }
        Expr::Unary(_, a) | Expr::Attr(a, _) => w(a),
        Expr::Compare(a, rest) => { w(a); rest.iter().for_each(|(_, x)| w(x)) }
        Expr::IfExp(a, b, c) => { w(a); w(b); w(c) }
        Expr::Call(func, args) => {
            w(func);
            for a in args {
                match a { Arg::Pos(x) | Arg::Star(x) | Arg::Kw(_, x) | Arg::StarStar(x) => w(x) }
            }
        }
        Expr::Slice(a, b, c) => [a, b, c].into_iter().flatten().for_each(|x| w(x)),
        Expr::Const(_) | Expr::Name(..) | Expr::Lambda(_) => {}
    }
}

/// Every name mentioned anywhere inside nested functions/lambdas of `body`.
/// Locals with these names become cells (a safe over-approximation).
fn names_in_nested(body: &[Stmt], out: &mut HashSet<Str>) {
    fn all_names_stmts(body: &[Stmt], out: &mut HashSet<Str>) {
        for s in body {
            all_names_stmt(s, out);
        }
    }
    fn all_names_expr(e: &Expr, out: &mut HashSet<Str>) {
        walk_expr(e, &mut |x| match x {
            Expr::Name(n, _) => { out.insert(n.clone()); }
            Expr::Lambda(f) => all_names_func(f, out),
            _ => {}
        });
    }
    fn all_names_func(f: &FuncDef, out: &mut HashSet<Str>) {
        for p in &f.params {
            if let Some(d) = &p.default {
                all_names_expr(d, out);
            }
        }
        all_names_stmts(&f.body, out);
    }
    fn all_names_stmt(s: &Stmt, out: &mut HashSet<Str>) {
        for_each_child(s, &mut |c| match c {
            Child::Expr(e) => all_names_expr(e, out),
            Child::Block(b) => all_names_stmts(b, out),
        });
        match &s.kind {
            StmtKind::Def(f, _) => all_names_func(f, out),
            StmtKind::Global(ns) | StmtKind::Nonlocal(ns) => out.extend(ns.iter().cloned()),
            _ => {}
        }
    }
    // Only names inside nested functions count
    fn nested(body: &[Stmt], out: &mut HashSet<Str>) {
        for s in body {
            for_each_child(s, &mut |c| match c {
                Child::Expr(e) => walk_expr(e, &mut |x| if let Expr::Lambda(f) = x { all_names_func(f, out) }),
                Child::Block(b) => nested(b, out),
            });
            if let StmtKind::Def(f, _) = &s.kind {
                all_names_func(f, out);
            }
        }
    }
    nested(body, out);
}

enum Child<'a> {
    Expr(&'a Expr),
    Block(&'a [Stmt]),
}

/// Visits the expressions and sub-blocks of one statement (not nested function bodies)
fn for_each_child<'a>(s: &'a Stmt, f: &mut dyn FnMut(Child<'a>)) {
    let fe = |e: &'a Expr, f: &mut dyn FnMut(Child<'a>)| f(Child::Expr(e));
    match &s.kind {
        StmtKind::Expr(e) => fe(e, f),
        StmtKind::Assign(ts, v) => {
            ts.iter().for_each(|t| fe(t, f));
            fe(v, f);
        }
        StmtKind::AugAssign(t, _, v) => { fe(t, f); fe(v, f) }
        StmtKind::If(branches, orelse) => {
            for (c, b) in branches {
                fe(c, f);
                f(Child::Block(b));
            }
            f(Child::Block(orelse));
        }
        StmtKind::While(c, b, o) => { fe(c, f); f(Child::Block(b)); f(Child::Block(o)) }
        StmtKind::For(t, i, b, o) => { fe(t, f); fe(i, f); f(Child::Block(b)); f(Child::Block(o)) }
        StmtKind::Def(def, t) => {
            def.params.iter().filter_map(|p| p.default.as_ref()).for_each(|d| fe(d, f));
            fe(t, f);
        }
        StmtKind::Class { target, base, body, .. } => {
            fe(target, f);
            if let Some(b) = base { fe(b, f) }
            f(Child::Block(body));
        }
        StmtKind::Return(Some(e)) | StmtKind::Raise(Some(e)) => fe(e, f),
        StmtKind::Try { body, handlers, orelse, finally } => {
            f(Child::Block(body));
            for h in handlers {
                if let Some(t) = &h.typ { fe(t, f) }
                if let Some(n) = &h.name { fe(n, f) }
                f(Child::Block(&h.body));
            }
            f(Child::Block(orelse));
            f(Child::Block(finally));
        }
        StmtKind::Del(ts) => ts.iter().for_each(|t| fe(t, f)),
        StmtKind::Assert(c, m) => { fe(c, f); if let Some(m) = m { fe(m, f) } }
        StmtKind::Import(items) | StmtKind::ImportFrom(_, items) => items.iter().for_each(|(_, t)| fe(t, f)),
        _ => {}
    }
}

// ---------- Resolution ----------

impl<'g> Resolver<'g> {
    fn cur(&mut self) -> &mut Scope {
        self.scopes.last_mut().unwrap()
    }

    /// Resolves a name in scope `depth` (index into self.scopes)
    fn lookup(&mut self, depth: usize, name: &Str) -> Var {
        let scope = &self.scopes[depth];
        match scope.kind {
            ScopeKind::Module => return Var::Global(self.globals.slot(name)),
            ScopeKind::Class => {
                if scope.class_names.contains(name) {
                    return Var::ClassNs(self.globals.slot(name));
                }
                // Otherwise the class body sees what its enclosing scope sees
                return self.lookup(depth - 1, name);
            }
            ScopeKind::Function => {}
        }
        if scope.globals.contains(name) {
            return Var::Global(self.globals.slot(name));
        }
        if let Some(&i) = scope.locals.get(name) {
            return Var::Local(i);
        }
        if let Some(&i) = scope.cells.get(name) {
            return Var::Cell(i);
        }
        if let Some(&i) = scope.frees.get(name) {
            return Var::Free(i);
        }
        // Find the nearest enclosing function that has it (class scopes are skipped)
        let mut d = depth;
        let mut found = false;
        while d > 0 {
            d -= 1;
            match self.scopes[d].kind {
                ScopeKind::Function => {
                    let s = &self.scopes[d];
                    if !s.globals.contains(name) && (s.cells.contains_key(name) || s.frees.contains_key(name) || s.locals.contains_key(name)) {
                        found = true;
                        break;
                    }
                }
                ScopeKind::Module => break,
                ScopeKind::Class => {}
            }
        }
        if !found {
            return Var::Global(self.globals.slot(name));
        }
        // Capture through the immediately enclosing function scope
        let mut parent = depth - 1;
        while matches!(self.scopes[parent].kind, ScopeKind::Class) {
            parent -= 1;
        }
        let outer = self.lookup(parent, name);
        let outer = match outer {
            Var::Local(_) => unreachable!("captured variables are cells"),
            v => v,
        };
        let scope = &mut self.scopes[depth];
        let i = scope.freevars.len() as u32;
        scope.freevars.push(outer);
        scope.frees.insert(name.clone(), i);
        Var::Free(i)
    }

    fn name(&mut self, n: &Str, var: &mut Var) {
        let depth = self.scopes.len() - 1;
        *var = self.lookup(depth, n);
    }

    fn block(&mut self, body: &mut [Stmt]) -> RResult {
        for s in body {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn stmt(&mut self, s: &mut Stmt) -> RResult {
        match &mut s.kind {
            StmtKind::Expr(e) => self.expr(e)?,
            StmtKind::Assign(ts, v) => {
                self.expr(v)?;
                for t in ts {
                    self.expr(t)?;
                }
            }
            StmtKind::AugAssign(t, _, v) => {
                self.expr(v)?;
                self.expr(t)?;
            }
            StmtKind::If(branches, orelse) => {
                for (c, b) in branches {
                    self.expr(c)?;
                    self.block(b)?;
                }
                self.block(orelse)?;
            }
            StmtKind::While(c, b, o) => {
                self.expr(c)?;
                self.block(b)?;
                self.block(o)?;
            }
            StmtKind::For(t, i, b, o) => {
                self.expr(i)?;
                self.expr(t)?;
                self.block(b)?;
                self.block(o)?;
            }
            StmtKind::Def(f, t) => {
                self.function(f)?;
                self.expr(t)?;
            }
            StmtKind::Class { target, base, body, .. } => {
                if let Some(b) = base {
                    self.expr(b)?;
                }
                let mut names = Vec::new();
                bound_names(body, &mut names, &mut HashSet::new(), &mut HashSet::new());
                self.scopes.push(Scope {
                    kind: ScopeKind::Class, locals: HashMap::new(), cells: HashMap::new(), frees: HashMap::new(),
                    freevars: Vec::new(), globals: HashSet::new(), class_names: names.into_iter().collect(),
                });
                self.block(body)?;
                self.scopes.pop();
                self.expr(target)?;
            }
            StmtKind::Return(Some(e)) | StmtKind::Raise(Some(e)) => self.expr(e)?,
            StmtKind::Try { body, handlers, orelse, finally } => {
                self.block(body)?;
                for h in handlers {
                    if let Some(t) = &mut h.typ {
                        self.expr(t)?;
                    }
                    if let Some(n) = &mut h.name {
                        self.expr(n)?;
                    }
                    self.block(&mut h.body)?;
                }
                self.block(orelse)?;
                self.block(finally)?;
            }
            StmtKind::Del(ts) => {
                for t in ts {
                    self.expr(t)?;
                }
            }
            StmtKind::Assert(c, m) => {
                self.expr(c)?;
                if let Some(m) = m {
                    self.expr(m)?;
                }
            }
            StmtKind::Import(items) | StmtKind::ImportFrom(_, items) => {
                for (_, t) in items {
                    self.expr(t)?;
                }
            }
            StmtKind::Nonlocal(names) => {
                for n in names.iter() {
                    if !matches!(self.cur().kind, ScopeKind::Function) || !self.cur().frees.contains_key(n) {
                        return Err(crate::lexer::err(s.line, format!("لا يوجد متغير '{n}' في دالة محيطة")));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn function(&mut self, f: &mut Rc<FuncDef>) -> RResult {
        let f = Rc::get_mut(f).expect("function definitions are resolved once");
        // Defaults are evaluated in the enclosing scope
        for p in &mut f.params {
            if let Some(d) = &mut p.default {
                self.expr(d)?;
            }
        }

        let mut names: Vec<Str> = f.params.iter().map(|p| p.name.clone()).collect();
        names.extend(f.vararg.iter().chain(&f.kwarg).map(|(n, _)| n.clone()));
        let (mut globals, mut nonlocals) = (HashSet::new(), HashSet::new());
        bound_names(&f.body, &mut names, &mut globals, &mut nonlocals);
        let mut captured = HashSet::new();
        names_in_nested(&f.body, &mut captured);

        let mut scope = Scope {
            kind: ScopeKind::Function, locals: HashMap::new(), cells: HashMap::new(), frees: HashMap::new(),
            freevars: Vec::new(), globals, class_names: HashSet::new(),
        };
        for n in names {
            if scope.globals.contains(&n) || nonlocals.contains(&n) || scope.locals.contains_key(&n) || scope.cells.contains_key(&n) {
                continue;
            }
            if captured.contains(&n) {
                let i = scope.cells.len() as u32;
                scope.cells.insert(n, i);
            } else {
                let i = scope.locals.len() as u32;
                scope.locals.insert(n, i);
            }
        }
        self.scopes.push(scope);
        // nonlocal names resolve through the enclosing functions
        let depth = self.scopes.len() - 1;
        for n in &nonlocals {
            let found = self.scopes[..depth].iter().any(|s| matches!(s.kind, ScopeKind::Function)
                && (s.locals.contains_key(n) || s.cells.contains_key(n) || s.frees.contains_key(n)));
            if !found {
                return Err(crate::lexer::err(f.line, format!("لا يوجد متغير '{n}' في دالة محيطة")));
            }
            self.lookup(depth, n);
        }

        for p in &mut f.params {
            p.var = self.lookup(depth, &p.name);
        }
        if let Some((n, v)) = &mut f.vararg {
            *v = self.lookup(depth, n);
        }
        if let Some((n, v)) = &mut f.kwarg {
            *v = self.lookup(depth, n);
        }
        self.block(&mut f.body)?;
        let scope = self.scopes.pop().unwrap();
        f.nlocals = scope.locals.len() as u32;
        f.simple_params = f.vararg.is_none() && f.kwarg.is_none()
            && f.params.iter().enumerate().all(|(i, p)| p.var == Var::Local(i as u32));
        f.ncells = scope.cells.len() as u32;
        f.freevars = scope.freevars;
        Ok(())
    }

    fn expr(&mut self, e: &mut Expr) -> RResult {
        match e {
            Expr::Name(n, var) => self.name(n, var),
            Expr::Const(_) => {}
            Expr::FString(parts) => {
                for p in parts {
                    if let FPart::Expr { expr, .. } = p {
                        self.expr(expr)?;
                    }
                }
            }
            Expr::List(items) | Expr::Tuple(items) => {
                for i in items {
                    self.expr(i)?;
                }
            }
            Expr::Dict(items) => {
                for (k, v) in items {
                    self.expr(k)?;
                    self.expr(v)?;
                }
            }
            Expr::ListComp(elt, comps) => {
                self.comps(comps)?;
                self.expr(elt)?;
            }
            Expr::DictComp(kv, comps) => {
                self.comps(comps)?;
                self.expr(&mut kv.0)?;
                self.expr(&mut kv.1)?;
            }
            Expr::Bin(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Index(a, b) => {
                self.expr(a)?;
                self.expr(b)?;
            }
            Expr::Unary(_, a) | Expr::Attr(a, _) => self.expr(a)?,
            Expr::Compare(a, rest) => {
                self.expr(a)?;
                for (_, x) in rest {
                    self.expr(x)?;
                }
            }
            Expr::IfExp(a, b, c) => {
                self.expr(a)?;
                self.expr(b)?;
                self.expr(c)?;
            }
            Expr::Call(func, args) => {
                self.expr(func)?;
                for a in args {
                    match a {
                        Arg::Pos(x) | Arg::Star(x) | Arg::Kw(_, x) | Arg::StarStar(x) => self.expr(x)?,
                    }
                }
            }
            Expr::Slice(a, b, c) => {
                for x in [a, b, c].into_iter().flatten() {
                    self.expr(x)?;
                }
            }
            Expr::Lambda(f) => self.function(f)?,
        }
        Ok(())
    }

    fn comps(&mut self, comps: &mut [Comp]) -> RResult {
        for c in comps {
            self.expr(&mut c.iter)?;
            self.expr(&mut c.target)?;
            for cond in &mut c.conds {
                self.expr(cond)?;
            }
        }
        Ok(())
    }
}
