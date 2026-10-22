// Runtime values, dictionaries, and the parts of str()/repr() that need no interpreter.

use crate::ast::FuncDef;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

/// FxHash: a fast non-cryptographic hasher (programs can't attack their own interpreter)
#[derive(Default, Clone, Copy)]
pub struct FxHasher(u64);

impl FxHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.add(u64::from_le_bytes(c.try_into().unwrap()));
        }
        for &b in chunks.remainder() {
            self.add(b as u64);
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

pub type Map<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

pub type Str = Rc<str>;
pub type Attrs = RefCell<AttrMap>;

/// Attributes of objects, classes and modules: few entries, so a vector beats hashing.
/// Names are interned by the lexer, so most lookups match on the pointer.
#[derive(Default, Clone)]
pub struct AttrMap(Vec<(Str, Value)>);

impl AttrMap {
    #[inline]
    fn find(&self, name: &str) -> Option<usize> {
        self.0.iter().position(|(k, _)| std::ptr::eq(k.as_ptr(), name.as_ptr()) && k.len() == name.len())
            .or_else(|| self.0.iter().position(|(k, _)| **k == *name))
    }

    #[inline]
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.find(name).map(|i| &self.0[i].1)
    }

    pub fn insert(&mut self, name: Str, v: Value) {
        match self.find(&name) {
            Some(i) => self.0[i].1 = v,
            None => self.0.push((name, v)),
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<Value> {
        self.find(name).map(|i| self.0.remove(i).1)
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.0.iter().map(|(_, v)| v)
    }
}

pub enum Value {
    /// An unbound variable slot; never visible to programs
    Undef,
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Str),
    List(Rc<RefCell<Vec<Value>>>),
    Tuple(Rc<Vec<Value>>),
    Dict(Rc<RefCell<Dict>>),
    Range(Rc<(i64, i64, i64)>),
    Func(Rc<Function>),
    Builtin(Builtin),
    /// A user method bound to its instance: (self, function)
    Method(Rc<(Value, Value)>),
    /// A built-in type's method bound to its receiver, e.g. [].append
    BuiltinMethod(Rc<(Value, Str)>),
    Class(Rc<Class>),
    Instance(Rc<Instance>),
    Module(Rc<Module>),
    /// super(): look up attributes after `.0` in the class chain, bound to `.1`
    Super(Rc<(Rc<Class>, Value)>),
}

impl Clone for Value {
    /// Numbers and constants are copied inline; reference types bump a count
    #[inline(always)]
    fn clone(&self) -> Value {
        match self {
            Value::Int(i) => Value::Int(*i),
            Value::Float(f) => Value::Float(*f),
            Value::Bool(b) => Value::Bool(*b),
            Value::None => Value::None,
            _ => self.clone_ref(),
        }
    }
}

impl Value {
    #[inline(never)]
    fn clone_ref(&self) -> Value {
        match self {
            Value::Undef => Value::Undef,
            Value::None => Value::None,
            Value::Bool(b) => Value::Bool(*b),
            Value::Int(i) => Value::Int(*i),
            Value::Float(f) => Value::Float(*f),
            Value::Str(s) => Value::Str(s.clone()),
            Value::List(l) => Value::List(l.clone()),
            Value::Tuple(t) => Value::Tuple(t.clone()),
            Value::Dict(d) => Value::Dict(d.clone()),
            Value::Range(r) => Value::Range(r.clone()),
            Value::Func(f) => Value::Func(f.clone()),
            Value::Builtin(b) => Value::Builtin(*b),
            Value::Method(m) => Value::Method(m.clone()),
            Value::BuiltinMethod(m) => Value::BuiltinMethod(m.clone()),
            Value::Class(c) => Value::Class(c.clone()),
            Value::Instance(i) => Value::Instance(i.clone()),
            Value::Module(m) => Value::Module(m.clone()),
            Value::Super(s) => Value::Super(s.clone()),
        }
    }
}

pub struct Function {
    pub def: Rc<FuncDef>,
    pub defaults: Vec<Value>,
    pub closure: Rc<Vec<Rc<RefCell<Value>>>>,
    /// Class whose body defined this function, for zero-argument super()
    pub owner: RefCell<Option<Rc<Class>>>,
}

pub struct Class {
    pub name: Str,
    pub base: Option<Rc<Class>>,
    pub attrs: Attrs,
}

pub struct Instance {
    pub class: Rc<Class>,
    pub attrs: Attrs,
}

pub struct Module {
    pub name: Str,
    pub attrs: Attrs,
}

impl Class {
    pub fn lookup(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.attrs.borrow().get(name) {
            return Some(v.clone());
        }
        self.base.as_ref().and_then(|b| b.lookup(name))
    }

    pub fn is_subclass_of(self: &Rc<Self>, other: &Rc<Class>) -> bool {
        let mut c = Some(self.clone());
        while let Some(cls) = c {
            if Rc::ptr_eq(&cls, other) {
                return true;
            }
            c = cls.base.clone();
        }
        false
    }
}

macro_rules! builtins {
    ($($id:ident = $name:literal),* $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Builtin { $($id),* }
        impl Builtin {
            pub fn name(self) -> &'static str {
                match self { $(Builtin::$id => $name),* }
            }
        }
    };
}

builtins! {
    // Types (callable as conversions)
    Int = "int", Float = "float", Str = "str", Bool = "bool", List = "list", Tuple = "tuple",
    Dict = "dict", Range = "range", Type = "type",
    // Functions
    Print = "print", Input = "input", Len = "len", Sum = "sum", Max = "max", Min = "min",
    Abs = "abs", Round = "round", Sorted = "sorted", Reversed = "reversed",
    Enumerate = "enumerate", Zip = "zip", Map = "map", Filter = "filter",
    Isinstance = "isinstance", Any = "any", All = "all", Chr = "chr", Ord = "ord",
    Repr = "repr", Divmod = "divmod", Pow = "pow", Super = "super", Hasattr = "hasattr",
    Getattr = "getattr", Setattr = "setattr", Format = "format",
    // math module
    MathSqrt = "sqrt", MathFloor = "floor", MathCeil = "ceil", MathSin = "sin", MathCos = "cos",
    MathTan = "tan", MathLog = "log", MathLog10 = "log10", MathExp = "exp", MathFabs = "fabs",
    MathFactorial = "factorial", MathGcd = "gcd", MathPow = "pow",
    // random module
    RandRandom = "random", RandRandint = "randint", RandChoice = "choice",
    RandShuffle = "shuffle", RandUniform = "uniform", RandSeed = "seed",
    // time module
    TimeTime = "time", TimeSleep = "sleep",
}

impl Builtin {
    pub fn is_type(self) -> bool {
        use Builtin::*;
        matches!(self, Int | Float | Str | Bool | List | Tuple | Dict | Range | Type)
    }
}

// ---------- Dict: insertion-ordered hash map ----------

/// Hashable form of a value: equal values (1, 1.0, True) give equal keys
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Key {
    None,
    Int(i64),
    Float(u64),
    Str(Str),
    Tuple(Vec<Key>),
    Ptr(usize), // classes, functions: identity
}

impl Key {
    pub fn of(v: &Value) -> Option<Key> {
        Some(match v {
            Value::None => Key::None,
            Value::Bool(b) => Key::Int(*b as i64),
            Value::Int(i) => Key::Int(*i),
            Value::Float(f) => {
                if f.fract() == 0.0 && f.abs() < 9.2e18 {
                    Key::Int(*f as i64)
                } else {
                    Key::Float(f.to_bits())
                }
            }
            Value::Str(s) => Key::Str(s.clone()),
            Value::Tuple(t) => Key::Tuple(t.iter().map(Key::of).collect::<Option<_>>()?),
            Value::Class(c) => Key::Ptr(Rc::as_ptr(c) as *const u8 as usize),
            Value::Func(f) => Key::Ptr(Rc::as_ptr(f) as *const u8 as usize),
            Value::Instance(i) => Key::Ptr(Rc::as_ptr(i) as *const u8 as usize),
            Value::Builtin(b) => Key::Ptr(*b as usize),
            _ => return None,
        })
    }
}

#[derive(Default, Clone)]
pub struct Dict {
    entries: Vec<Option<(Value, Value)>>,
    index: Map<Key, usize>,
}

impl Dict {
    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn get(&self, k: &Key) -> Option<&Value> {
        self.index.get(k).and_then(|&i| self.entries[i].as_ref()).map(|(_, v)| v)
    }

    pub fn insert(&mut self, key: Key, k: Value, v: Value) {
        match self.index.get(&key) {
            Some(&i) => self.entries[i].as_mut().unwrap().1 = v,
            None => {
                self.index.insert(key, self.entries.len());
                self.entries.push(Some((k, v)));
            }
        }
    }

    pub fn remove(&mut self, key: &Key) -> Option<Value> {
        let i = self.index.remove(key)?;
        let v = self.entries[i].take().map(|(_, v)| v);
        if self.entries.len() > 32 && self.index.len() * 2 < self.entries.len() {
            self.compact();
        }
        v
    }

    fn compact(&mut self) {
        self.entries.retain(Option::is_some);
        for (i, (k, _)) in self.entries.iter().flatten().enumerate() {
            self.index.insert(Key::of(k).unwrap(), i);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Value, &Value)> {
        self.entries.iter().flatten().map(|(k, v)| (k, v))
    }

    pub fn pop_last(&mut self) -> Option<(Value, Value)> {
        while let Some(e) = self.entries.pop() {
            if let Some((k, v)) = e {
                self.index.remove(&Key::of(&k).unwrap());
                return Some((k, v));
            }
        }
        None
    }
}

// ---------- Helpers ----------

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(s.into())
    }

    pub fn list(v: Vec<Value>) -> Value {
        Value::List(Rc::new(RefCell::new(v)))
    }

    pub fn tuple(v: Vec<Value>) -> Value {
        Value::Tuple(Rc::new(v))
    }

    #[inline]
    pub fn truthy(&self) -> bool {
        match self {
            Value::Undef | Value::None => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.borrow().is_empty(),
            Value::Tuple(t) => !t.is_empty(),
            Value::Dict(d) => d.borrow().len() > 0,
            Value::Range(r) => range_len(r) > 0,
            _ => true,
        }
    }

    /// Arabic type name, used in error messages
    pub fn type_name(&self) -> String {
        match self {
            Value::Undef | Value::None => "لاشيء".into(),
            Value::Bool(_) => "قيمة_منطقية".into(),
            Value::Int(_) => "عدد_صحيح".into(),
            Value::Float(_) => "عدد_عشري".into(),
            Value::Str(_) => "نص".into(),
            Value::List(_) => "قائمة".into(),
            Value::Tuple(_) => "صف".into(),
            Value::Dict(_) => "قاموس".into(),
            Value::Range(_) => "نطاق".into(),
            Value::Func(_) | Value::Method(_) => "دالة".into(),
            Value::Builtin(b) if b.is_type() => "صنف".into(),
            Value::Builtin(_) | Value::BuiltinMethod(_) => "دالة_مدمجة".into(),
            Value::Class(_) => "صنف".into(),
            Value::Instance(i) => i.class.name.to_string(),
            Value::Module(_) => "وحدة".into(),
            Value::Super(_) => "super".into(),
        }
    }

    /// The built-in type of a value, for type() and isinstance()
    pub fn builtin_type(&self) -> Option<Builtin> {
        Some(match self {
            Value::Bool(_) => Builtin::Bool,
            Value::Int(_) => Builtin::Int,
            Value::Float(_) => Builtin::Float,
            Value::Str(_) => Builtin::Str,
            Value::List(_) => Builtin::List,
            Value::Tuple(_) => Builtin::Tuple,
            Value::Dict(_) => Builtin::Dict,
            Value::Range(_) => Builtin::Range,
            Value::Class(_) | Value::Builtin(_) => return None,
            _ => return None,
        })
    }
}

pub fn range_len(r: &(i64, i64, i64)) -> i64 {
    let (start, stop, step) = *r;
    if step > 0 && start < stop {
        (stop - start - 1) / step + 1
    } else if step < 0 && start > stop {
        (start - stop - 1) / (-step) + 1
    } else {
        0
    }
}

/// Python-style float repr: shortest round-trip digits, scientific outside 1e-4..1e16
pub fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let sci = format!("{:e}", f); // e.g. "-1.2345e6"
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    if exp < -4 || exp >= 16 {
        s.push_str(&digits[..1]);
        if digits.len() > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        let _ = write!(s, "e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    } else if exp < 0 {
        s.push_str("0.");
        s.push_str(&"0".repeat((-exp - 1) as usize));
        s.push_str(&digits);
    } else {
        let int_len = exp as usize + 1;
        if digits.len() <= int_len {
            s.push_str(&digits);
            s.push_str(&"0".repeat(int_len - digits.len()));
            s.push_str(".0");
        } else {
            s.push_str(&digits[..int_len]);
            s.push('.');
            s.push_str(&digits[int_len..]);
        }
    }
    s
}

pub fn str_repr(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

/// Equality without user-defined __eq__ (instances compare by identity)
pub fn values_eq(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (None, None) => true,
        (Bool(x), Bool(y)) => x == y,
        (Int(_) | Bool(_) | Float(_), Int(_) | Bool(_) | Float(_)) => match (num(a), num(b)) {
            (Num::I(x), Num::I(y)) => x == y,
            (x, y) => x.f() == y.f(),
        },
        (Str(x), Str(y)) => x == y,
        (List(x), List(y)) => Rc::ptr_eq(x, y) || seq_eq(&x.borrow(), &y.borrow()),
        (Tuple(x), Tuple(y)) => seq_eq(x, y),
        (Dict(x), Dict(y)) => {
            let (x, y) = (x.borrow(), y.borrow());
            x.len() == y.len()
                && x.iter().all(|(k, v)| y.get(&Key::of(k).unwrap()).is_some_and(|w| values_eq(v, w)))
        }
        (Range(x), Range(y)) => x == y,
        (Func(x), Func(y)) => Rc::ptr_eq(x, y),
        (Builtin(x), Builtin(y)) => x == y,
        (Class(x), Class(y)) => Rc::ptr_eq(x, y),
        (Instance(x), Instance(y)) => Rc::ptr_eq(x, y),
        (Module(x), Module(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

fn seq_eq(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| values_eq(x, y))
}

#[derive(Clone, Copy)]
pub enum Num {
    I(i64),
    F(f64),
}

impl Num {
    pub fn f(self) -> f64 {
        match self {
            Num::I(i) => i as f64,
            Num::F(f) => f,
        }
    }
}

/// Numeric view of a value; panics on non-numbers, so check first
pub fn num(v: &Value) -> Num {
    match v {
        Value::Int(i) => Num::I(*i),
        Value::Bool(b) => Num::I(*b as i64),
        Value::Float(f) => Num::F(*f),
        _ => unreachable!(),
    }
}

pub fn is_num(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Bool(_) | Value::Float(_))
}

/// Ordering for <, sorted(), max(): numbers, strings, and sequences of those
pub fn compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    use Value::*;
    match (a, b) {
        _ if is_num(a) && is_num(b) => match (num(a), num(b)) {
            (Num::I(x), Num::I(y)) => Some(x.cmp(&y)),
            (x, y) => x.f().partial_cmp(&y.f()),
        },
        (Str(x), Str(y)) => Some(x.cmp(y)),
        (List(x), List(y)) => seq_cmp(&x.borrow(), &y.borrow()),
        (Tuple(x), Tuple(y)) => seq_cmp(x, y),
        _ => Option::None,
    }
}

fn seq_cmp(a: &[Value], b: &[Value]) -> Option<std::cmp::Ordering> {
    for (x, y) in a.iter().zip(b) {
        if !values_eq(x, y) {
            return compare(x, y);
        }
    }
    Some(a.len().cmp(&b.len()))
}
