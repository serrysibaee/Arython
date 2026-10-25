// Built-in functions, methods of built-in types, modules, and format specs.

use crate::interp::{Interp, R};
use crate::lexer::digit_value;
use crate::value::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Builtins available everywhere without import
pub const GLOBAL: &[Builtin] = {
    use Builtin::*;
    &[
        Int, Float, Str, Bool, List, Tuple, Dict, Range, Type, Print, Input, Len, Sum, Max, Min,
        Abs, Round, Sorted, Reversed, Enumerate, Zip, Map, Filter, Isinstance, Any, All, Chr, Ord,
        Repr, Divmod, Pow, Super, Hasattr, Getattr, Setattr, Format,
    ]
};

pub fn module(name: &str) -> Option<AttrMap> {
    use Builtin::*;
    let (fns, consts): (&[Builtin], Vec<(&str, Value)>) = match name {
        "math" => (
            &[MathSqrt, MathFloor, MathCeil, MathSin, MathCos, MathTan, MathLog, MathLog10, MathExp,
              MathFabs, MathFactorial, MathGcd, MathPow],
            vec![("pi", Value::Float(std::f64::consts::PI)), ("e", Value::Float(std::f64::consts::E)),
                 ("tau", Value::Float(std::f64::consts::TAU)), ("inf", Value::Float(f64::INFINITY))],
        ),
        "random" => (&[RandRandom, RandRandint, RandChoice, RandShuffle, RandUniform, RandSeed], vec![]),
        "time" => (&[TimeTime, TimeSleep], vec![]),
        _ => return None,
    };
    let mut m = AttrMap::default();
    for b in fns {
        m.insert(b.name().into(), Value::Builtin(*b));
    }
    for (k, v) in consts {
        m.insert(k.into(), v);
    }
    Some(m)
}

// ---------- Argument helpers ----------

struct Args<'a> {
    name: &'a str,
    pos: Vec<Value>,
    kw: Vec<(Str, Value)>,
}

impl<'a> Args<'a> {
    fn count(&self, it: &Interp, min: usize, max: usize) -> R<()> {
        let n = self.pos.len();
        if n < min || n > max {
            let expected = if min == max { format!("{min}") } else if max == usize::MAX { format!("{min} على الأقل") } else { format!("من {min} إلى {max}") };
            return it.type_err(format!("{}() تأخذ {expected} وسائط، وأُعطيت {n}", self.name));
        }
        Ok(())
    }

    fn kw(&mut self, name: &str) -> Option<Value> {
        let i = self.kw.iter().position(|(k, _)| &**k == name)?;
        Some(self.kw.remove(i).1)
    }

    fn no_kw(&self, it: &Interp) -> R<()> {
        match self.kw.first() {
            Some((k, _)) => it.type_err(format!("{}() لا تقبل الوسيط '{k}'", self.name)),
            None => Ok(()),
        }
    }
}

fn int_arg(it: &Interp, v: &Value, what: &str) -> R<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Bool(b) => Ok(*b as i64),
        _ => it.type_err(format!("{what} يجب أن يكون عددا صحيحا، لا {}", v.type_name())),
    }
}

fn float_arg(it: &Interp, v: &Value, what: &str) -> R<f64> {
    if is_num(v) {
        Ok(num(v).f())
    } else {
        it.type_err(format!("{what} يجب أن يكون رقما، لا {}", v.type_name()))
    }
}

fn str_arg<'v>(it: &Interp, v: &'v Value, what: &str) -> R<&'v Str> {
    match v {
        Value::Str(s) => Ok(s),
        _ => it.type_err(format!("{what} يجب أن يكون نصا، لا {}", v.type_name())),
    }
}

/// Converts Arabic-Indic digits and the Arabic decimal separator to ASCII
fn ascii_number(s: &str) -> String {
    s.trim().chars().map(|c| match digit_value(c) {
        Some(d) => char::from_digit(d, 10).unwrap(),
        None if c == '٫' => '.',
        None => c,
    }).filter(|c| *c != '_').collect()
}

fn parse_int(s: &str, base: u32) -> Option<i64> {
    let t = ascii_number(s);
    let (neg, digits) = match t.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, t.strip_prefix('+').unwrap_or(&t)),
    };
    let digits = match base {
        16 => digits.trim_start_matches("0x").trim_start_matches("0X"),
        8 => digits.trim_start_matches("0o").trim_start_matches("0O"),
        2 => digits.trim_start_matches("0b").trim_start_matches("0B"),
        _ => digits,
    };
    if digits.is_empty() {
        return None;
    }
    let v = i64::from_str_radix(digits, base).ok()?;
    Some(if neg { -v } else { v })
}

fn parse_float(s: &str) -> Option<f64> {
    let t = ascii_number(s).to_lowercase();
    match t.as_str() {
        "inf" | "+inf" | "infinity" => Some(f64::INFINITY),
        "-inf" | "-infinity" => Some(f64::NEG_INFINITY),
        "nan" | "+nan" | "-nan" => Some(f64::NAN),
        _ if t.chars().all(|c| c.is_ascii_digit() || "+-.e".contains(c)) => t.parse().ok(),
        _ => None,
    }
}

/// Python's round(): half to even, at `digits` decimal places
fn round_float(x: f64, digits: i64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    if digits >= 0 {
        // Formatting rounds the exact binary value correctly, like CPython
        format!("{:.*}", digits.min(300) as usize, x).parse().unwrap_or(x)
    } else {
        let p = 10f64.powi((-digits) as i32);
        (x / p).round_ties_even() * p
    }
}

// ---------- Builtin functions ----------

pub fn call_builtin(it: &mut Interp, b: Builtin, pos: Vec<Value>, kw: Vec<(Str, Value)>) -> R<Value> {
    use Builtin as B;
    let mut a = Args { name: b.name(), pos, kw };
    // print and the key=/reverse= functions take keywords; the rest don't
    if !matches!(b, B::Print | B::Sorted | B::Max | B::Min | B::Dict | B::Enumerate | B::Sum | B::Round | B::Int) {
        a.no_kw(it)?;
    }
    Ok(match b {
        B::Print => {
            let sep = match a.kw("sep") { Some(Value::None) | None => " ".into(), Some(v) => it.to_str(&v)? };
            let end = match a.kw("end") { Some(Value::None) | None => "\n".into(), Some(v) => it.to_str(&v)? };
            a.kw("flush");
            a.no_kw(it)?;
            let mut s = String::new();
            for (i, v) in a.pos.iter().enumerate() {
                if i > 0 {
                    s.push_str(&sep);
                }
                s.push_str(&it.to_str(v)?);
            }
            s.push_str(&end);
            it.write(&s);
            Value::None
        }
        B::Input => {
            a.count(it, 0, 1)?;
            let prompt = match a.pos.first() { Some(v) => it.to_str(v)?, None => String::new() };
            Value::Str(it.read_line(&prompt)?.into())
        }
        B::Len => {
            a.count(it, 1, 1)?;
            let v = &a.pos[0];
            Value::Int(match v {
                Value::Str(s) => s.chars().count() as i64,
                Value::List(l) => l.borrow().len() as i64,
                Value::Tuple(t) => t.len() as i64,
                Value::Dict(d) => d.borrow().len() as i64,
                Value::Range(r) => range_len(r),
                Value::Instance(inst) => match inst.class.lookup("__len__") {
                    Some(f) => return it.call(&f, vec![v.clone()], Vec::new()),
                    None => return it.type_err(format!("{} ليس له طول", v.type_name())),
                },
                _ => return it.type_err(format!("{} ليس له طول", v.type_name())),
            })
        }
        B::Int => {
            let base = a.kw("base");
            a.no_kw(it)?;
            a.count(it, 0, 2)?;
            let base = match a.pos.get(1).cloned().or(base) {
                Some(v) => Some(int_arg(it, &v, "الأساس")?),
                None => None,
            };
            match (a.pos.first(), base) {
                (None, _) => Value::Int(0),
                (Some(Value::Str(s)), base) => {
                    let base = base.unwrap_or(10);
                    if !(2..=36).contains(&base) {
                        return Err(it.err("ValueError", "الأساس يجب أن يكون بين 2 و36"));
                    }
                    match parse_int(s, base as u32) {
                        Some(i) => Value::Int(i),
                        None => return Err(it.err("ValueError", format!("لا يمكن تحويل {} إلى عدد صحيح", str_repr(s)))),
                    }
                }
                (Some(_), Some(_)) => return it.type_err("int() بأساس تقبل النصوص فقط"),
                (Some(Value::Int(i)), _) => Value::Int(*i),
                (Some(Value::Bool(b)), _) => Value::Int(*b as i64),
                (Some(Value::Float(f)), _) => {
                    if f.is_nan() {
                        return Err(it.err("ValueError", "لا يمكن تحويل nan إلى عدد صحيح"));
                    }
                    if !f.is_finite() || f.abs() >= 9.2e18 {
                        return Err(it.overflow());
                    }
                    Value::Int(f.trunc() as i64)
                }
                (Some(v), _) => return it.type_err(format!("لا يمكن تحويل {} إلى عدد صحيح", v.type_name())),
            }
        }
        B::Float => {
            a.count(it, 0, 1)?;
            match a.pos.first() {
                None => Value::Float(0.0),
                Some(Value::Str(s)) => match parse_float(s) {
                    Some(f) => Value::Float(f),
                    None => return Err(it.err("ValueError", format!("لا يمكن تحويل {} إلى عدد عشري", str_repr(s)))),
                },
                Some(v) if is_num(v) => Value::Float(num(v).f()),
                Some(v) => return it.type_err(format!("لا يمكن تحويل {} إلى عدد عشري", v.type_name())),
            }
        }
        B::Str => {
            a.count(it, 0, 1)?;
            match a.pos.first() {
                None => Value::str(""),
                Some(Value::Int(i)) => Value::Str(itoa(*i)),
                Some(v) => Value::Str(it.to_str(v)?.into()),
            }
        }
        B::Bool => {
            a.count(it, 0, 1)?;
            Value::Bool(a.pos.first().is_some_and(|v| v.truthy()))
        }
        B::List => {
            a.count(it, 0, 1)?;
            Value::list(match a.pos.first() { Some(v) => it.iterate(v)?, None => Vec::new() })
        }
        B::Tuple => {
            a.count(it, 0, 1)?;
            match a.pos.first() {
                Some(Value::Tuple(t)) => Value::Tuple(t.clone()),
                Some(v) => Value::tuple(it.iterate(v)?),
                None => Value::tuple(Vec::new()),
            }
        }
        B::Dict => {
            a.count(it, 0, 1)?;
            let mut d = Dict::default();
            if let Some(src) = a.pos.first() {
                dict_update(it, &mut d, src)?;
            }
            for (k, v) in std::mem::take(&mut a.kw) {
                d.insert(Key::Str(k.clone()), Value::Str(k), v);
            }
            Value::Dict(Rc::new(RefCell::new(d)))
        }
        B::Range => {
            a.count(it, 1, 3)?;
            let n: Vec<i64> = a.pos.iter().map(|v| int_arg(it, v, "حدود النطاق")).collect::<R<_>>()?;
            let (start, stop, step) = match n.len() {
                1 => (0, n[0], 1),
                2 => (n[0], n[1], 1),
                _ => (n[0], n[1], n[2]),
            };
            if step == 0 {
                return Err(it.err("ValueError", "خطوة النطاق لا يمكن أن تكون صفرا"));
            }
            Value::Range(Rc::new((start, stop, step)))
        }
        B::Type => {
            a.count(it, 1, 1)?;
            let v = &a.pos[0];
            match v {
                Value::Instance(i) => Value::Class(i.class.clone()),
                _ => match v.builtin_type() {
                    Some(t) => Value::Builtin(t),
                    None => Value::Builtin(B::Type),
                },
            }
        }
        B::Sum => {
            let start = a.kw("start");
            a.no_kw(it)?;
            a.count(it, 1, 2)?;
            let mut total = a.pos.get(1).cloned().or(start).unwrap_or(Value::Int(0));
            for v in it.iterate(&a.pos[0])? {
                total = it.binop(crate::ast::BinOp::Add, total, v)?;
            }
            total
        }
        B::Max | B::Min => {
            let key = a.kw("key");
            let default = a.kw("default");
            a.no_kw(it)?;
            a.count(it, 1, usize::MAX)?;
            let items = if a.pos.len() == 1 { it.iterate(&a.pos[0])? } else { a.pos.clone() };
            if items.is_empty() {
                return match default {
                    Some(d) => Ok(d),
                    None => Err(it.err("ValueError", format!("{}() أُعطيت تسلسلا فارغا", a.name))),
                };
            }
            let mut best = items[0].clone();
            let mut best_key = keyed(it, &key, &best)?;
            for v in items.into_iter().skip(1) {
                let k = keyed(it, &key, &v)?;
                let op = if b == B::Max { crate::ast::CmpOp::Gt } else { crate::ast::CmpOp::Lt };
                if it.compare_op(op, &k, &best_key)? {
                    best = v;
                    best_key = k;
                }
            }
            best
        }
        B::Abs => {
            a.count(it, 1, 1)?;
            match &a.pos[0] {
                Value::Int(i) => Value::Int(i.checked_abs().ok_or_else(|| it.overflow())?),
                Value::Bool(b) => Value::Int(*b as i64),
                Value::Float(f) => Value::Float(f.abs()),
                v => return it.type_err(format!("abs() لا تعمل على {}", v.type_name())),
            }
        }
        B::Round => {
            let nd = a.kw("ndigits");
            a.no_kw(it)?;
            a.count(it, 1, 2)?;
            let digits = match a.pos.get(1).cloned().or(nd) {
                Some(Value::None) | None => None,
                Some(v) => Some(int_arg(it, &v, "عدد الخانات")?),
            };
            match (&a.pos[0], digits) {
                (Value::Int(i), None) => Value::Int(*i),
                (Value::Bool(b), None) => Value::Int(*b as i64),
                (Value::Int(i), Some(d)) if d >= 0 => Value::Int(*i),
                (Value::Int(i), Some(d)) => {
                    let p = 10i64.checked_pow((-d) as u32).unwrap_or(i64::MAX);
                    Value::Int(round_float(*i as f64, d) as i64 / p * p)
                }
                (Value::Float(f), None) => {
                    let r = f.round_ties_even();
                    if !r.is_finite() || r.abs() >= 9.2e18 {
                        return Err(it.overflow());
                    }
                    Value::Int(r as i64)
                }
                (Value::Float(f), Some(d)) => Value::Float(round_float(*f, d)),
                (v, _) => return it.type_err(format!("round() لا تعمل على {}", v.type_name())),
            }
        }
        B::Sorted => {
            let key = a.kw("key");
            let reverse = a.kw("reverse").is_some_and(|v| v.truthy());
            a.no_kw(it)?;
            a.count(it, 1, 1)?;
            let mut items = it.iterate(&a.pos[0])?;
            sort_values(it, &mut items, &key, reverse)?;
            Value::list(items)
        }
        B::Reversed => {
            a.count(it, 1, 1)?;
            let mut items = it.iterate(&a.pos[0])?;
            items.reverse();
            Value::list(items)
        }
        B::Enumerate => {
            let start = a.kw("start");
            a.no_kw(it)?;
            a.count(it, 1, 2)?;
            let start = match a.pos.get(1).cloned().or(start) {
                Some(v) => int_arg(it, &v, "البداية")?,
                None => 0,
            };
            let items = it.iterate(&a.pos[0])?;
            Value::list(items.into_iter().enumerate().map(|(i, v)| Value::tuple(vec![Value::Int(start + i as i64), v])).collect())
        }
        B::Zip => {
            let lists: Vec<Vec<Value>> = a.pos.iter().map(|v| it.iterate(v)).collect::<R<_>>()?;
            let n = lists.iter().map(Vec::len).min().unwrap_or(0);
            Value::list((0..n).map(|i| Value::tuple(lists.iter().map(|l| l[i].clone()).collect())).collect())
        }
        B::Map => {
            a.count(it, 2, usize::MAX)?;
            let f = a.pos[0].clone();
            let lists: Vec<Vec<Value>> = a.pos[1..].iter().map(|v| it.iterate(v)).collect::<R<_>>()?;
            let n = lists.iter().map(Vec::len).min().unwrap_or(0);
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                out.push(it.call(&f, lists.iter().map(|l| l[i].clone()).collect(), Vec::new())?);
            }
            Value::list(out)
        }
        B::Filter => {
            a.count(it, 2, 2)?;
            let f = a.pos[0].clone();
            let mut out = Vec::new();
            for v in it.iterate(&a.pos[1])? {
                let keep = match &f {
                    Value::None => v.truthy(),
                    f => it.call(f, vec![v.clone()], Vec::new())?.truthy(),
                };
                if keep {
                    out.push(v);
                }
            }
            Value::list(out)
        }
        B::Isinstance => {
            a.count(it, 2, 2)?;
            Value::Bool(isinstance(it, &a.pos[0], &a.pos[1])?)
        }
        B::Any | B::All => {
            a.count(it, 1, 1)?;
            let items = it.iterate(&a.pos[0])?;
            Value::Bool(if b == B::Any { items.iter().any(Value::truthy) } else { items.iter().all(Value::truthy) })
        }
        B::Chr => {
            a.count(it, 1, 1)?;
            let i = int_arg(it, &a.pos[0], "رمز الحرف")?;
            match u32::try_from(i).ok().and_then(char::from_u32) {
                Some(c) => Value::Str(c.to_string().into()),
                None => return Err(it.err("ValueError", "رمز الحرف خارج النطاق")),
            }
        }
        B::Ord => {
            a.count(it, 1, 1)?;
            let s = str_arg(it, &a.pos[0], "وسيط ord()")?;
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Value::Int(c as i64),
                _ => return it.type_err("ord() تأخذ حرفا واحدا"),
            }
        }
        B::Repr => {
            a.count(it, 1, 1)?;
            Value::Str(it.repr(&a.pos[0])?.into())
        }
        B::Divmod => {
            a.count(it, 2, 2)?;
            let q = it.binop(crate::ast::BinOp::FloorDiv, a.pos[0].clone(), a.pos[1].clone())?;
            let r = it.binop(crate::ast::BinOp::Mod, a.pos[0].clone(), a.pos[1].clone())?;
            Value::tuple(vec![q, r])
        }
        B::Pow | B::MathPow => {
            a.count(it, 2, 3)?;
            if let Some(m) = a.pos.get(2) {
                let (mut base, mut exp, m) = (int_arg(it, &a.pos[0], "الأساس")?, int_arg(it, &a.pos[1], "الأس")?, int_arg(it, m, "المقياس")?);
                if m == 0 {
                    return Err(it.err("ValueError", "المقياس لا يمكن أن يكون صفرا"));
                }
                if exp < 0 {
                    return Err(it.err("ValueError", "الأس السالب غير مدعوم مع المقياس"));
                }
                let m = m as i128;
                let mut result: i128 = 1;
                let mut bb = (base as i128).rem_euclid(m);
                while exp > 0 {
                    if exp & 1 == 1 {
                        result = result * bb % m;
                    }
                    bb = bb * bb % m;
                    exp >>= 1;
                }
                base = result.rem_euclid(m) as i64;
                Value::Int(base)
            } else if b == B::MathPow {
                Value::Float(float_arg(it, &a.pos[0], "الأساس")?.powf(float_arg(it, &a.pos[1], "الأس")?))
            } else {
                it.binop(crate::ast::BinOp::Pow, a.pos[0].clone(), a.pos[1].clone())?
            }
        }
        B::Super => {
            a.count(it, 2, 2)?;
            match &a.pos[0] {
                Value::Class(c) => Value::Super(Rc::new((c.clone(), a.pos[1].clone()))),
                _ => return it.type_err("super() تحتاج صنفا"),
            }
        }
        B::Hasattr => {
            a.count(it, 2, 2)?;
            let name = str_arg(it, &a.pos[1], "اسم الخاصية")?.clone();
            match it.getattr(&a.pos[0], &name) {
                Ok(_) => Value::Bool(true),
                Err(e) if matches!(&e.value, Value::Instance(i) if &*i.class.name == "AttributeError") => Value::Bool(false),
                Err(e) => return Err(e),
            }
        }
        B::Getattr => {
            a.count(it, 2, 3)?;
            let name = str_arg(it, &a.pos[1], "اسم الخاصية")?.clone();
            match (it.getattr(&a.pos[0], &name), a.pos.get(2)) {
                (Ok(v), _) => v,
                (Err(_), Some(d)) => d.clone(),
                (Err(e), None) => return Err(e),
            }
        }
        B::Setattr => {
            a.count(it, 3, 3)?;
            let name = str_arg(it, &a.pos[1], "اسم الخاصية")?.clone();
            it.setattr(&a.pos[0], &name, a.pos[2].clone())?;
            Value::None
        }
        B::Format => {
            a.count(it, 1, 2)?;
            let spec = match a.pos.get(1) { Some(v) => str_arg(it, v, "صيغة التنسيق")?.clone(), None => "".into() };
            Value::Str(format_value(it, &a.pos[0], &spec)?.into())
        }
        // math
        B::MathSqrt | B::MathSin | B::MathCos | B::MathTan | B::MathLog | B::MathLog10 | B::MathExp | B::MathFabs => {
            a.count(it, 1, if b == B::MathLog { 2 } else { 1 })?;
            let x = float_arg(it, &a.pos[0], "الوسيط")?;
            let domain = || it.err("ValueError", "قيمة خارج مجال الدالة الرياضية");
            Value::Float(match b {
                B::MathSqrt if x < 0.0 => return Err(domain()),
                B::MathSqrt => x.sqrt(),
                B::MathSin => x.sin(),
                B::MathCos => x.cos(),
                B::MathTan => x.tan(),
                B::MathLog | B::MathLog10 if x <= 0.0 => return Err(domain()),
                B::MathLog => match a.pos.get(1) {
                    Some(base) => x.ln() / float_arg(it, base, "الأساس")?.ln(),
                    None => x.ln(),
                },
                B::MathLog10 => x.log10(),
                B::MathExp => x.exp(),
                _ => x.abs(),
            })
        }
        B::MathFloor | B::MathCeil => {
            a.count(it, 1, 1)?;
            match &a.pos[0] {
                Value::Int(i) => Value::Int(*i),
                v => {
                    let x = float_arg(it, v, "الوسيط")?;
                    let r = if b == B::MathFloor { x.floor() } else { x.ceil() };
                    if !r.is_finite() || r.abs() >= 9.2e18 {
                        return Err(it.overflow());
                    }
                    Value::Int(r as i64)
                }
            }
        }
        B::MathFactorial => {
            a.count(it, 1, 1)?;
            let n = int_arg(it, &a.pos[0], "الوسيط")?;
            if n < 0 {
                return Err(it.err("ValueError", "المضروب غير معرّف للأعداد السالبة"));
            }
            let mut r: i64 = 1;
            for k in 2..=n {
                r = r.checked_mul(k).ok_or_else(|| it.overflow())?;
            }
            Value::Int(r)
        }
        B::MathGcd => {
            let mut g: i64 = 0;
            for v in &a.pos {
                let mut x = int_arg(it, v, "الوسيط")?.abs();
                let mut y = g;
                while y != 0 {
                    (x, y) = (y, x % y);
                }
                g = x;
            }
            Value::Int(g)
        }
        // random
        B::RandRandom => {
            a.count(it, 0, 0)?;
            Value::Float(next_f64(it))
        }
        B::RandUniform => {
            a.count(it, 2, 2)?;
            let (lo, hi) = (float_arg(it, &a.pos[0], "الحد")?, float_arg(it, &a.pos[1], "الحد")?);
            Value::Float(lo + (hi - lo) * next_f64(it))
        }
        B::RandRandint => {
            a.count(it, 2, 2)?;
            let (lo, hi) = (int_arg(it, &a.pos[0], "الحد")?, int_arg(it, &a.pos[1], "الحد")?);
            if hi < lo {
                return Err(it.err("ValueError", "الحد الأعلى أصغر من الحد الأدنى"));
            }
            Value::Int(lo + (next_u64(it) % ((hi - lo) as u64 + 1)) as i64)
        }
        B::RandChoice => {
            a.count(it, 1, 1)?;
            let items = it.iterate(&a.pos[0])?;
            if items.is_empty() {
                return Err(it.err("IndexError", "لا يمكن الاختيار من تسلسل فارغ"));
            }
            let i = (next_u64(it) % items.len() as u64) as usize;
            items[i].clone()
        }
        B::RandShuffle => {
            a.count(it, 1, 1)?;
            let Value::List(l) = &a.pos[0] else {
                return it.type_err("shuffle() تعمل على القوائم فقط");
            };
            let n = l.borrow().len();
            for i in (1..n).rev() {
                let j = (next_u64(it) % (i as u64 + 1)) as usize;
                l.borrow_mut().swap(i, j);
            }
            Value::None
        }
        B::RandSeed => {
            a.count(it, 0, 1)?;
            it.rng = match a.pos.first() {
                Some(v) => (int_arg(it, v, "البذرة")? as u64).wrapping_mul(0x9E3779B97F4A7C15) | 1,
                None => 0x2545F4914F6CDD1D,
            };
            Value::None
        }
        // time
        B::TimeTime => {
            a.count(it, 0, 0)?;
            Value::Float(crate::host::now_secs())
        }
        B::TimeSleep => {
            a.count(it, 1, 1)?;
            let s = float_arg(it, &a.pos[0], "المدة")?;
            it.flush();
            if s > 0.0 && !crate::host::sleep(s) {
                return Err(it.err("KeyboardInterrupt", ""));
            }
            Value::None
        }
    })
}

/// Decimal text of an integer, allocated once
fn itoa(i: i64) -> Str {
    let mut buf = [0u8; 21];
    let mut n = i.unsigned_abs();
    let mut pos = buf.len();
    loop {
        pos -= 1;
        buf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    if i < 0 {
        pos -= 1;
        buf[pos] = b'-';
    }
    std::str::from_utf8(&buf[pos..]).unwrap().into()
}

fn keyed(it: &mut Interp, key: &Option<Value>, v: &Value) -> R<Value> {
    match key {
        Some(Value::None) | None => Ok(v.clone()),
        Some(f) => it.call(f, vec![v.clone()], Vec::new()),
    }
}

fn sort_values(it: &mut Interp, items: &mut Vec<Value>, key: &Option<Value>, reverse: bool) -> R<()> {
    let keys: Vec<Value> = items.iter().map(|v| keyed(it, key, v)).collect::<R<_>>()?;
    let mut idx: Vec<usize> = (0..items.len()).collect();
    let mut failed = None;
    idx.sort_by(|&x, &y| {
        let (a, b) = if reverse { (&keys[y], &keys[x]) } else { (&keys[x], &keys[y]) };
        compare(a, b).unwrap_or_else(|| {
            failed.get_or_insert((a.type_name(), b.type_name()));
            std::cmp::Ordering::Equal
        })
    });
    if let Some((a, b)) = failed {
        return it.type_err(format!("لا يمكن ترتيب قيم من النوعين {a} و{b} معا"));
    }
    let sorted: Vec<Value> = idx.into_iter().map(|i| items[i].clone()).collect();
    *items = sorted;
    Ok(())
}

fn isinstance(it: &Interp, v: &Value, t: &Value) -> R<bool> {
    Ok(match t {
        Value::Tuple(ts) => {
            for t in ts.iter() {
                if isinstance(it, v, t)? {
                    return Ok(true);
                }
            }
            false
        }
        Value::Class(c) => matches!(v, Value::Instance(i) if i.class.is_subclass_of(c)),
        Value::Builtin(Builtin::Int) => matches!(v, Value::Int(_) | Value::Bool(_)),
        Value::Builtin(b) if b.is_type() => v.builtin_type() == Some(*b),
        _ => return it.type_err("الوسيط الثاني لـ isinstance() يجب أن يكون صنفا"),
    })
}

fn dict_update(it: &mut Interp, d: &mut Dict, src: &Value) -> R<()> {
    if let Value::Dict(other) = src {
        let pairs: Vec<(Value, Value)> = other.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (k, v) in pairs {
            d.insert(it.key(&k)?, k, v);
        }
        return Ok(());
    }
    for pair in it.iterate(src)? {
        let kv = it.iterate(&pair)?;
        if kv.len() != 2 {
            return Err(it.err("ValueError", "كل عنصر يجب أن يكون زوجا (مفتاح، قيمة)"));
        }
        d.insert(it.key(&kv[0])?, kv[0].clone(), kv[1].clone());
    }
    Ok(())
}

fn next_u64(it: &mut Interp) -> u64 {
    // xorshift64*
    let mut x = it.rng;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    it.rng = x;
    x.wrapping_mul(0x2545F4914F6CDD1D)
}

fn next_f64(it: &mut Interp) -> f64 {
    (next_u64(it) >> 11) as f64 / (1u64 << 53) as f64
}

// ---------- Methods of built-in types ----------

const STR_METHODS: &[&str] = &[
    "upper", "lower", "strip", "lstrip", "rstrip", "split", "join", "replace", "startswith",
    "endswith", "find", "rfind", "index", "count", "format", "isdigit", "isalpha", "isalnum",
    "isspace", "isupper", "islower", "title", "capitalize", "center", "ljust", "rjust", "zfill",
    "splitlines", "swapcase",
];
const LIST_METHODS: &[&str] = &[
    "append", "extend", "insert", "pop", "remove", "index", "count", "sort", "reverse", "clear", "copy",
];
const DICT_METHODS: &[&str] = &["items", "keys", "values", "get", "pop", "update", "setdefault", "clear", "copy", "popitem"];
const TUPLE_METHODS: &[&str] = &["index", "count"];

pub fn has_method(v: &Value, name: &str) -> bool {
    let list = match v {
        Value::Str(_) => STR_METHODS,
        Value::List(_) => LIST_METHODS,
        Value::Dict(_) => DICT_METHODS,
        Value::Tuple(_) => TUPLE_METHODS,
        Value::Float(_) => &["is_integer"],
        _ => return false,
    };
    list.contains(&name)
}

pub fn call_method(it: &mut Interp, obj: &Value, name: &str, pos: Vec<Value>, kw: Vec<(Str, Value)>) -> R<Value> {
    let mut a = Args { name, pos, kw };
    match obj {
        Value::Str(s) => str_method(it, s, &mut a),
        Value::List(l) => list_method(it, obj, l, &mut a),
        Value::Dict(d) => dict_method(it, d, &mut a),
        Value::Tuple(t) => {
            a.no_kw(it)?;
            a.count(it, 1, 1)?;
            seq_index_count(it, t, &a)
        }
        Value::Float(f) => {
            a.count(it, 0, 0)?;
            Ok(Value::Bool(f.fract() == 0.0))
        }
        _ => Err(it.no_attr(obj, name)),
    }
}

fn seq_index_count(it: &mut Interp, items: &[Value], a: &Args) -> R<Value> {
    let target = &a.pos[0];
    if a.name == "count" {
        let mut n = 0;
        for v in items {
            if it.eq(v, target)? {
                n += 1;
            }
        }
        return Ok(Value::Int(n));
    }
    for (i, v) in items.iter().enumerate() {
        if it.eq(v, target)? {
            return Ok(Value::Int(i as i64));
        }
    }
    let r = it.repr(target)?;
    Err(it.err("ValueError", format!("{r} غير موجود")))
}

fn str_method(it: &mut Interp, s: &Str, a: &mut Args) -> R<Value> {
    let sv = |x: String| Ok(Value::Str(x.into()));
    if a.name != "format" && a.name != "split" {
        a.no_kw(it)?;
    }
    match a.name {
        "upper" => { a.count(it, 0, 0)?; sv(s.to_uppercase()) }
        "lower" => { a.count(it, 0, 0)?; sv(s.to_lowercase()) }
        "swapcase" => {
            a.count(it, 0, 0)?;
            sv(s.chars().map(|c| if c.is_uppercase() { c.to_lowercase().collect::<String>() } else { c.to_uppercase().collect() }).collect())
        }
        "title" => {
            a.count(it, 0, 0)?;
            let mut out = String::new();
            let mut prev_letter = false;
            for c in s.chars() {
                if c.is_alphabetic() {
                    if prev_letter { out.extend(c.to_lowercase()) } else { out.extend(c.to_uppercase()) }
                    prev_letter = true;
                } else {
                    out.push(c);
                    prev_letter = false;
                }
            }
            sv(out)
        }
        "capitalize" => {
            a.count(it, 0, 0)?;
            let mut chars = s.chars();
            sv(match chars.next() {
                Some(c) => c.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect(),
                None => String::new(),
            })
        }
        "strip" | "lstrip" | "rstrip" => {
            a.count(it, 0, 1)?;
            let chars: Option<Vec<char>> = match a.pos.first() {
                Some(Value::None) | None => None,
                Some(v) => Some(str_arg(it, v, "الحروف")?.chars().collect()),
            };
            let pred = |c: char| match &chars { Some(cs) => cs.contains(&c), None => c.is_whitespace() };
            sv(match a.name {
                "strip" => s.trim_matches(pred),
                "lstrip" => s.trim_start_matches(pred),
                _ => s.trim_end_matches(pred),
            }.to_string())
        }
        "split" => {
            let sep_kw = a.kw("sep");
            let max_kw = a.kw("maxsplit");
            a.no_kw(it)?;
            a.count(it, 0, 2)?;
            let sep = match a.pos.first().cloned().or(sep_kw) {
                Some(Value::None) | None => None,
                Some(v) => Some(str_arg(it, &v, "الفاصل")?.clone()),
            };
            let max = match a.pos.get(1).cloned().or(max_kw) {
                Some(v) => int_arg(it, &v, "الحد")?,
                None => -1,
            };
            let parts: Vec<String> = match &sep {
                Some(sep) if sep.is_empty() => return Err(it.err("ValueError", "الفاصل فارغ")),
                Some(sep) if max < 0 => s.split(&**sep).map(String::from).collect(),
                Some(sep) => s.splitn(max as usize + 1, &**sep).map(String::from).collect(),
                None => {
                    let mut out: Vec<String> = Vec::new();
                    let mut rest = s.trim_start();
                    while !rest.is_empty() {
                        if max >= 0 && out.len() as i64 == max {
                            out.push(rest.to_string());
                            break;
                        }
                        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                        out.push(rest[..end].to_string());
                        rest = rest[end..].trim_start();
                    }
                    out
                }
            };
            Ok(Value::list(parts.into_iter().map(|p| Value::Str(p.into())).collect()))
        }
        "splitlines" => {
            a.count(it, 0, 0)?;
            Ok(Value::list(s.lines().map(Value::str).collect()))
        }
        "join" => {
            a.count(it, 1, 1)?;
            let items = it.iterate(&a.pos[0])?;
            let mut out = String::new();
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(s);
                }
                match v {
                    Value::Str(x) => out.push_str(x),
                    v => return it.type_err(format!("join() تتوقع نصوصا، لكن وجدت {} (استخدم نص() للتحويل)", v.type_name())),
                }
            }
            sv(out)
        }
        "replace" => {
            a.count(it, 2, 3)?;
            let old = str_arg(it, &a.pos[0], "النص القديم")?.clone();
            let new = str_arg(it, &a.pos[1], "النص الجديد")?.clone();
            match a.pos.get(2) {
                Some(n) => {
                    let n = int_arg(it, n, "العدد")?;
                    sv(if n < 0 { s.replace(&*old, &new) } else { s.replacen(&*old, &new, n as usize) })
                }
                None => sv(s.replace(&*old, &new)),
            }
        }
        "startswith" | "endswith" => {
            a.count(it, 1, 1)?;
            let check = |p: &str| if a.name == "startswith" { s.starts_with(p) } else { s.ends_with(p) };
            Ok(Value::Bool(match &a.pos[0] {
                Value::Tuple(t) => t.iter().any(|p| matches!(p, Value::Str(p) if check(p))),
                v => check(str_arg(it, v, "البادئة")?),
            }))
        }
        "find" | "rfind" | "index" | "count" => {
            a.count(it, 1, 1)?;
            let sub = str_arg(it, &a.pos[0], "النص المبحوث عنه")?;
            if a.name == "count" {
                return Ok(Value::Int(if sub.is_empty() { s.chars().count() as i64 + 1 } else { s.matches(&**sub).count() as i64 }));
            }
            let byte = if a.name == "rfind" { s.rfind(&**sub) } else { s.find(&**sub) };
            match byte {
                Some(b) => Ok(Value::Int(s[..b].chars().count() as i64)),
                None if a.name == "index" => Err(it.err("ValueError", "النص الفرعي غير موجود")),
                None => Ok(Value::Int(-1)),
            }
        }
        "isdigit" | "isalpha" | "isalnum" | "isspace" | "isupper" | "islower" => {
            a.count(it, 0, 0)?;
            let ok = match a.name {
                "isdigit" => !s.is_empty() && s.chars().all(|c| digit_value(c).is_some()),
                "isalpha" => !s.is_empty() && s.chars().all(char::is_alphabetic),
                "isalnum" => !s.is_empty() && s.chars().all(char::is_alphanumeric),
                "isspace" => !s.is_empty() && s.chars().all(char::is_whitespace),
                "isupper" => s.chars().any(char::is_uppercase) && !s.chars().any(char::is_lowercase),
                _ => s.chars().any(char::is_lowercase) && !s.chars().any(char::is_uppercase),
            };
            Ok(Value::Bool(ok))
        }
        "center" | "ljust" | "rjust" => {
            a.count(it, 1, 2)?;
            let width = int_arg(it, &a.pos[0], "العرض")?.max(0) as usize;
            let fill = match a.pos.get(1) {
                Some(v) => str_arg(it, v, "حرف الملء")?.chars().next().unwrap_or(' '),
                None => ' ',
            };
            let align = match a.name { "center" => '^', "ljust" => '<', _ => '>' };
            sv(pad(s, width, fill, align))
        }
        "zfill" => {
            a.count(it, 1, 1)?;
            let width = int_arg(it, &a.pos[0], "العرض")?.max(0) as usize;
            let len = s.chars().count();
            if len >= width {
                return sv(s.to_string());
            }
            let (sign, rest) = match s.chars().next() {
                Some(c @ ('-' | '+')) => (c.to_string(), &s[1..]),
                _ => (String::new(), &**s),
            };
            sv(format!("{sign}{}{rest}", "0".repeat(width - len)))
        }
        "format" => {
            let kw = std::mem::take(&mut a.kw);
            sv(str_format(it, s, &a.pos, &kw)?)
        }
        _ => Err(it.no_attr(&Value::Str(s.clone()), a.name)),
    }
}

fn list_method(it: &mut Interp, obj: &Value, l: &Rc<RefCell<Vec<Value>>>, a: &mut Args) -> R<Value> {
    if a.name != "sort" {
        a.no_kw(it)?;
    }
    match a.name {
        "append" => {
            a.count(it, 1, 1)?;
            l.borrow_mut().push(a.pos.pop().unwrap());
        }
        "extend" => {
            a.count(it, 1, 1)?;
            let items = it.iterate(&a.pos[0])?;
            l.borrow_mut().extend(items);
        }
        "insert" => {
            a.count(it, 2, 2)?;
            let i = int_arg(it, &a.pos[0], "الموضع")?;
            let len = l.borrow().len() as i64;
            let i = if i < 0 { (i + len).max(0) } else { i.min(len) } as usize;
            l.borrow_mut().insert(i, a.pos[1].clone());
        }
        "pop" => {
            a.count(it, 0, 1)?;
            let len = l.borrow().len();
            if len == 0 {
                return Err(it.err("IndexError", "لا يمكن الحذف من قائمة فارغة"));
            }
            let i = match a.pos.first() {
                Some(v) => it.index(v, len, "القائمة")?,
                None => len - 1,
            };
            return Ok(l.borrow_mut().remove(i));
        }
        "remove" => {
            a.count(it, 1, 1)?;
            let items = l.borrow().clone();
            for (i, v) in items.iter().enumerate() {
                if it.eq(v, &a.pos[0])? {
                    l.borrow_mut().remove(i);
                    return Ok(Value::None);
                }
            }
            let r = it.repr(&a.pos[0])?;
            return Err(it.err("ValueError", format!("{r} غير موجود في القائمة")));
        }
        "index" | "count" => {
            a.count(it, 1, 1)?;
            let items = l.borrow().clone();
            return seq_index_count(it, &items, a);
        }
        "sort" => {
            let key = a.kw("key");
            let reverse = a.kw("reverse").is_some_and(|v| v.truthy());
            a.no_kw(it)?;
            a.count(it, 0, 0)?;
            let mut items = l.borrow().clone();
            sort_values(it, &mut items, &key, reverse)?;
            *l.borrow_mut() = items;
        }
        "reverse" => {
            a.count(it, 0, 0)?;
            l.borrow_mut().reverse();
        }
        "clear" => {
            a.count(it, 0, 0)?;
            l.borrow_mut().clear();
        }
        "copy" => {
            a.count(it, 0, 0)?;
            return Ok(Value::list(l.borrow().clone()));
        }
        _ => return Err(it.no_attr(obj, a.name)),
    }
    Ok(Value::None)
}

fn dict_method(it: &mut Interp, d: &Rc<RefCell<Dict>>, a: &mut Args) -> R<Value> {
    if a.name != "update" {
        a.no_kw(it)?;
    }
    Ok(match a.name {
        "items" => {
            a.count(it, 0, 0)?;
            Value::list(d.borrow().iter().map(|(k, v)| Value::tuple(vec![k.clone(), v.clone()])).collect())
        }
        "keys" => {
            a.count(it, 0, 0)?;
            Value::list(d.borrow().iter().map(|(k, _)| k.clone()).collect())
        }
        "values" => {
            a.count(it, 0, 0)?;
            Value::list(d.borrow().iter().map(|(_, v)| v.clone()).collect())
        }
        "get" => {
            a.count(it, 1, 2)?;
            let key = it.key(&a.pos[0])?;
            let found = d.borrow().get(&key).cloned();
            found.unwrap_or_else(|| a.pos.get(1).cloned().unwrap_or(Value::None))
        }
        "pop" => {
            a.count(it, 1, 2)?;
            let key = it.key(&a.pos[0])?;
            let removed = d.borrow_mut().remove(&key);
            match (removed, a.pos.get(1)) {
                (Some(v), _) => v,
                (None, Some(default)) => default.clone(),
                (None, None) => return Err(it.key_error(&a.pos[0])),
            }
        }
        "popitem" => {
            a.count(it, 0, 0)?;
            let last = d.borrow_mut().pop_last();
            match last {
                Some((k, v)) => Value::tuple(vec![k, v]),
                None => return Err(it.err("KeyError", "القاموس فارغ")),
            }
        }
        "setdefault" => {
            a.count(it, 1, 2)?;
            let key = it.key(&a.pos[0])?;
            let existing = d.borrow().get(&key).cloned();
            match existing {
                Some(v) => v,
                None => {
                    let v = a.pos.get(1).cloned().unwrap_or(Value::None);
                    d.borrow_mut().insert(key, a.pos[0].clone(), v.clone());
                    v
                }
            }
        }
        "update" => {
            a.count(it, 0, 1)?;
            let mut copy = d.borrow().clone();
            if let Some(src) = a.pos.first() {
                dict_update(it, &mut copy, src)?;
            }
            for (k, v) in std::mem::take(&mut a.kw) {
                copy.insert(Key::Str(k.clone()), Value::Str(k), v);
            }
            *d.borrow_mut() = copy;
            Value::None
        }
        "clear" => {
            a.count(it, 0, 0)?;
            d.borrow_mut().clear();
            Value::None
        }
        "copy" => {
            a.count(it, 0, 0)?;
            Value::Dict(Rc::new(RefCell::new(d.borrow().clone())))
        }
        _ => return Err(it.no_attr(&Value::Dict(d.clone()), a.name)),
    })
}

// ---------- Formatting ----------

fn pad(s: &str, width: usize, fill: char, align: char) -> String {
    let len = s.chars().count();
    if len >= width {
        return s.to_string();
    }
    let n = width - len;
    let f = |k: usize| fill.to_string().repeat(k);
    match align {
        '<' => format!("{s}{}", f(n)),
        '^' => format!("{}{s}{}", f(n / 2), f(n - n / 2)),
        _ => format!("{}{s}", f(n)),
    }
}

fn group_thousands(digits: &str, sep: char) -> String {
    let (int, frac) = match digits.find('.') {
        Some(i) => (&digits[..i], &digits[i..]),
        None => (digits, ""),
    };
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    out + frac
}

/// 'g' formatting: `p` significant digits, scientific for very large/small values
fn general(x: f64, p: usize, alt: bool, upper: bool) -> String {
    if !x.is_finite() {
        return float_repr(x);
    }
    let p = p.max(1);
    if x == 0.0 {
        return if alt { format!("{:.*}", p - 1, 0.0) } else { "0".into() };
    }
    let sci = format!("{:.*e}", p - 1, x);
    let exp: i32 = sci.split_once('e').unwrap().1.parse().unwrap();
    let trim = |s: String| -> String {
        if alt || !s.contains('.') {
            return s;
        }
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    if exp >= -4 && exp < p as i32 {
        trim(format!("{:.*}", (p as i32 - 1 - exp) as usize, x))
    } else {
        let (m, e) = sci.split_once('e').unwrap();
        let e: i32 = e.parse().unwrap();
        let s = format!("{}e{}{:02}", trim(m.to_string()), if e < 0 { '-' } else { '+' }, e.abs());
        if upper { s.to_uppercase() } else { s }
    }
}

/// Applies a format spec ([[fill]align][sign][#][0][width][,][.precision][type]) to a value
pub fn format_value(it: &mut Interp, v: &Value, spec: &str) -> R<String> {
    if spec.is_empty() {
        return it.to_str(v);
    }
    let chars: Vec<char> = spec.chars().collect();
    let mut i = 0;
    let mut fill = ' ';
    let mut align = None;
    if chars.len() >= 2 && "<>^=".contains(chars[1]) {
        fill = chars[0];
        align = Some(chars[1]);
        i = 2;
    } else if !chars.is_empty() && "<>^=".contains(chars[0]) {
        align = Some(chars[0]);
        i = 1;
    }
    let mut sign = '-';
    if i < chars.len() && "+- ".contains(chars[i]) {
        sign = chars[i];
        i += 1;
    }
    let mut alt = false;
    if i < chars.len() && chars[i] == '#' {
        alt = true;
        i += 1;
    }
    if i < chars.len() && chars[i] == '0' && align.is_none() {
        fill = '0';
        align = Some('=');
        i += 1;
    }
    let mut width = 0usize;
    while i < chars.len() && chars[i].is_ascii_digit() {
        width = width * 10 + chars[i].to_digit(10).unwrap() as usize;
        i += 1;
    }
    let mut grouping = None;
    if i < chars.len() && (chars[i] == ',' || chars[i] == '_') {
        grouping = Some(chars[i]);
        i += 1;
    }
    let mut precision = None;
    if i < chars.len() && chars[i] == '.' {
        i += 1;
        let mut p = 0usize;
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            p = p * 10 + chars[i].to_digit(10).unwrap() as usize;
            i += 1;
        }
        if i == start {
            return Err(it.err("ValueError", format!("صيغة تنسيق غير صالحة: '{spec}'")));
        }
        precision = Some(p);
    }
    let ty = if i < chars.len() { Some(chars[i]) } else { None };
    if i + 1 < chars.len() {
        return Err(it.err("ValueError", format!("صيغة تنسيق غير صالحة: '{spec}'")));
    }

    let bad = |it: &Interp| Err(it.err("ValueError", format!("الصيغة '{spec}' لا تناسب {}", v.type_name())));
    let numeric = is_num(v) && !matches!(ty, Some('s'));
    let (neg, body) = if numeric {
        let x = num(v);
        let neg = x.f().is_sign_negative() && !(x.f() == 0.0 && matches!(x, Num::I(_))) && !x.f().is_nan();
        let body = match (ty, x) {
            (None | Some('d' | 'n'), Num::I(i)) if precision.is_none() => i.unsigned_abs().to_string(),
            (Some('d'), Num::F(_)) => return bad(it),
            (Some('x'), Num::I(i)) => format!("{}{:x}", if alt { "0x" } else { "" }, i.unsigned_abs()),
            (Some('X'), Num::I(i)) => format!("{}{:X}", if alt { "0X" } else { "" }, i.unsigned_abs()),
            (Some('o'), Num::I(i)) => format!("{}{:o}", if alt { "0o" } else { "" }, i.unsigned_abs()),
            (Some('b'), Num::I(i)) => format!("{}{:b}", if alt { "0b" } else { "" }, i.unsigned_abs()),
            (Some('x' | 'X' | 'o' | 'b' | 'c'), _) => return bad(it),
            (Some('f' | 'F'), x) => {
                let f = x.f().abs();
                if f.is_finite() { format!("{:.*}", precision.unwrap_or(6), f) } else { float_repr(f) }
            }
            (Some('e' | 'E'), x) => {
                let s = format!("{:.*e}", precision.unwrap_or(6), x.f().abs());
                let (m, e) = s.split_once('e').unwrap();
                let e: i32 = e.parse().unwrap();
                let s = format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
                if ty == Some('E') { s.to_uppercase() } else { s }
            }
            (Some('%'), x) => format!("{:.*}%", precision.unwrap_or(6), x.f().abs() * 100.0),
            (Some('g' | 'G'), x) => general(x.f().abs(), precision.unwrap_or(6), alt, ty == Some('G')),
            (None | Some('n'), Num::F(f)) => match precision {
                Some(p) => general(f.abs(), p, alt, false),
                None => float_repr(f.abs()),
            },
            (None | Some('n'), Num::I(i)) => general(i.unsigned_abs() as f64, precision.unwrap_or(6), alt, false),
            (Some(_), _) => return bad(it),
        };
        (neg, body)
    } else {
        if !matches!(ty, None | Some('s')) || grouping.is_some() || sign != '-' {
            return bad(it);
        }
        let mut s = it.to_str(v)?;
        if let Some(p) = precision {
            s = s.chars().take(p).collect();
        }
        (false, s)
    };
    let body = match grouping {
        Some(g) if numeric && body.chars().next().is_some_and(|c| c.is_ascii_digit()) => group_thousands(&body, g),
        _ => body,
    };
    let sign_str = match (neg, sign) {
        (true, _) => "-",
        (false, '+') => "+",
        (false, ' ') => " ",
        _ => "",
    };
    let align = align.unwrap_or(if numeric { '>' } else { '<' });
    if align == '=' {
        let n = width.saturating_sub(sign_str.chars().count());
        return Ok(format!("{sign_str}{}", pad(&body, n, fill, '>')));
    }
    Ok(pad(&format!("{sign_str}{body}"), width, fill, align))
}

/// str.format(): {} {0} {name} with optional !conv and :spec
fn str_format(it: &mut Interp, fmt: &str, pos: &[Value], kw: &[(Str, Value)]) -> R<String> {
    let chars: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut auto = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if (c == '{' || c == '}') && chars.get(i + 1) == Some(&c) {
            out.push(c);
            i += 2;
            continue;
        }
        if c == '}' {
            return Err(it.err("ValueError", "قوس '}' منفرد في نص التنسيق"));
        }
        if c != '{' {
            out.push(c);
            i += 1;
            continue;
        }
        let end = match chars[i..].iter().position(|&x| x == '}') {
            Some(e) => i + e,
            None => return Err(it.err("ValueError", "قوس '{' غير مغلق في نص التنسيق")),
        };
        let field: String = chars[i + 1..end].iter().collect();
        let (field, spec) = field.split_once(':').map_or((field.as_str(), ""), |(a, b)| (a, b));
        let (field, conv) = field.split_once('!').map_or((field, None), |(a, b)| (a, b.chars().next()));
        let v = if field.is_empty() {
            auto += 1;
            pos.get(auto - 1).cloned()
        } else if let Ok(n) = field.parse::<usize>() {
            pos.get(n).cloned()
        } else {
            kw.iter().find(|(k, _)| &**k == field).map(|(_, v)| v.clone())
        };
        let Some(v) = v else {
            return Err(it.err("IndexError", format!("لا توجد قيمة للحقل '{{{field}}}' في format()")));
        };
        let v = match conv {
            Some('r') => Value::Str(it.repr(&v)?.into()),
            Some('s') => Value::Str(it.to_str(&v)?.into()),
            _ => v,
        };
        out.push_str(&format_value(it, &v, spec)?);
        i = end + 1;
    }
    Ok(out)
}

/// printf-style "%s %d %.2f" % values
pub fn percent_format(it: &mut Interp, fmt: &str, args: &Value) -> R<String> {
    let values: Vec<Value> = match args {
        Value::Tuple(t) => t.to_vec(),
        v => vec![v.clone()],
    };
    let chars: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut next = 0;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        i += 1;
        if chars.get(i) == Some(&'%') {
            out.push('%');
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_ascii_alphabetic() {
            i += 1;
        }
        let Some(&ty) = chars.get(i) else {
            return Err(it.err("ValueError", "صيغة % غير مكتملة"));
        };
        let flags: String = chars[start..i].iter().collect();
        i += 1;
        let Some(v) = values.get(next).cloned() else {
            return it.type_err("عدد القيم أقل من المطلوب في التنسيق بـ %");
        };
        next += 1;
        let s = match ty {
            's' => {
                let s = it.to_str(&v)?;
                format_value(it, &Value::Str(s.into()), &flags.replace('-', "<"))?
            }
            'r' => {
                let s = it.repr(&v)?;
                format_value(it, &Value::Str(s.into()), &flags)?
            }
            'd' | 'i' => {
                let iv = match &v {
                    Value::Float(f) => Value::Int(f.trunc() as i64),
                    v => v.clone(),
                };
                format_value(it, &iv, &format!("{}d", flags.replace('-', "<")))?
            }
            'f' | 'e' | 'g' | 'x' | 'o' => format_value(it, &v, &format!("{}{ty}", flags.replace('-', "<")))?,
            _ => return Err(it.err("ValueError", format!("نوع تنسيق غير مدعوم: %{ty}"))),
        };
        out.push_str(&s);
    }
    if next < values.len() && matches!(args, Value::Tuple(_)) {
        return it.type_err("عدد القيم أكثر من المطلوب في التنسيق بـ %");
    }
    Ok(out)
}
