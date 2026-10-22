// Syntax tree. Variable references start Unresolved; resolve.rs fills in where each lives.

use crate::value::{Str, Value};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Var {
    Unresolved,
    /// Slot in the current function's locals
    Local(u32),
    /// Local captured by an inner function: lives in a shared cell
    Cell(u32),
    /// Variable of an enclosing function, captured in this function's closure
    Free(u32),
    /// Module-level variable (or builtin)
    Global(u32),
    /// Name assigned in a class body: class namespace first, then this global slot
    ClassNs(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div, FloorDiv, Mod, Pow, BitAnd, BitOr, BitXor, Shl, Shr,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CmpOp {
    Eq, Ne, Lt, Le, Gt, Ge, In, NotIn, Is, IsNot,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnOp {
    Neg, Pos, Not, Invert,
}

pub enum FPart {
    Lit(Str),
    Expr { expr: Expr, conv: Option<char>, spec: Str },
}

pub enum Arg {
    Pos(Expr),
    Star(Expr),
    Kw(Str, Expr),
    StarStar(Expr),
}

pub struct Comp {
    pub target: Expr,
    pub iter: Expr,
    pub conds: Vec<Expr>,
}

pub enum Expr {
    Const(Value),
    Name(Str, Var),
    FString(Vec<FPart>),
    List(Vec<Expr>),
    Tuple(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    ListComp(Box<Expr>, Vec<Comp>),
    DictComp(Box<(Expr, Expr)>, Vec<Comp>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Compare(Box<Expr>, Vec<(CmpOp, Expr)>),
    IfExp(Box<Expr>, Box<Expr>, Box<Expr>), // body, cond, orelse
    Call(Box<Expr>, Vec<Arg>),
    Attr(Box<Expr>, Str),
    Index(Box<Expr>, Box<Expr>),
    Slice(Option<Box<Expr>>, Option<Box<Expr>>, Option<Box<Expr>>),
    Lambda(Rc<FuncDef>),
}

pub struct Param {
    pub name: Str,
    pub var: Var,
    pub default: Option<Expr>,
}

pub struct FuncDef {
    pub name: Str,
    pub params: Vec<Param>,
    pub vararg: Option<(Str, Var)>,
    pub kwarg: Option<(Str, Var)>,
    pub body: Vec<Stmt>,
    pub line: u32,
    // Filled in by the resolver
    pub nlocals: u32,
    /// Parameters occupy local slots 0..n in order, with no *args/**kwargs
    pub simple_params: bool,
    pub ncells: u32,
    /// Where each closure variable comes from in the enclosing scope (Cell or Free)
    pub freevars: Vec<Var>,
}

pub struct Handler {
    pub typ: Option<Expr>,
    pub name: Option<Expr>,
    pub body: Vec<Stmt>,
}

pub enum StmtKind {
    Expr(Expr),
    Assign(Vec<Expr>, Expr),
    AugAssign(Expr, BinOp, Expr),
    If(Vec<(Expr, Vec<Stmt>)>, Vec<Stmt>),
    While(Expr, Vec<Stmt>, Vec<Stmt>),
    For(Expr, Expr, Vec<Stmt>, Vec<Stmt>),
    Def(Rc<FuncDef>, Expr),
    Class { name: Str, target: Expr, base: Option<Expr>, body: Vec<Stmt> },
    Return(Option<Expr>),
    Pass,
    Break,
    Continue,
    Global(Vec<Str>),
    Nonlocal(Vec<Str>),
    Raise(Option<Expr>),
    Try { body: Vec<Stmt>, handlers: Vec<Handler>, orelse: Vec<Stmt>, finally: Vec<Stmt> },
    Del(Vec<Expr>),
    Assert(Expr, Option<Expr>),
    Import(Vec<(Str, Expr)>),
    ImportFrom(Str, Vec<(Str, Expr)>),
}

pub struct Stmt {
    pub line: u32,
    pub kind: StmtKind,
}
