// Lexer: Arabic source → tokens. Arabic keywords, digits and punctuation are
// mapped here, so the parser only ever sees one canonical vocabulary.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

thread_local! {
    static NAMES: RefCell<HashSet<Rc<str>>> = RefCell::new(HashSet::new());
}

/// One shared Rc per distinct name, so attribute lookups can compare pointers
pub fn intern(s: &str) -> Rc<str> {
    NAMES.with(|n| {
        let mut n = n.borrow_mut();
        if let Some(r) = n.get(s) {
            return r.clone();
        }
        let r: Rc<str> = s.into();
        n.insert(r.clone());
        r
    })
}

/// Arabic words and the Python words they mean. Keep in sync with
/// ARABIC_TO_PYTHON in ../arabic_translator.py so both runtimes agree.
pub const ARABIC_WORDS: &[(&str, &str)] = &[
    // Keywords
    ("اطبع", "print"), ("إذا", "if"), ("وإلا_إذا", "elif"), ("وإلا", "else"),
    ("بينما", "while"), ("لكل", "for"), ("في", "in"), ("في_ضمن", "in"),
    ("دالة", "def"), ("أرجع", "return"), ("استمرار", "continue"), ("اكسر", "break"),
    ("حاول", "try"), ("عدا", "except"), ("أخيرا", "finally"), ("مع", "with"),
    ("كـ", "as"), ("استيراد", "import"), ("من", "from"), ("صحيح", "True"),
    ("خطأ", "False"), ("لاشيء", "None"), ("و", "and"), ("أو", "or"),
    ("غير", "not"), ("صنف", "class"), ("يمر", "pass"), ("عالمي", "global"),
    ("غير_محلي", "nonlocal"), ("رفع", "raise"), ("هو", "is"),
    // Built-in functions
    ("أدخل", "input"), ("طول", "len"), ("نوع", "type"), ("نطاق", "range"),
    ("عدد_صحيح", "int"), ("عدد_عشري", "float"), ("نص", "str"), ("قائمة", "list"),
    ("مجموع", "sum"), ("أكبر", "max"), ("أصغر", "min"), ("مطلق", "abs"),
    ("قرّب", "round"), ("مرتب", "sorted"), ("قاموس", "dict"), ("عدّد", "enumerate"),
    // Common methods
    ("أضف", "append"), ("عناصر", "items"), ("مفاتيح", "keys"), ("قيم", "values"),
    ("افصل", "split"), ("ادمج", "join"), ("كبير", "upper"),
    // Word operators
    ("يساوي", "=="), ("لا_يساوي", "!="), ("أكبر_من", ">"), ("أصغر_من", "<"),
    ("أكبر_أو_يساوي", ">="), ("أصغر_أو_يساوي", "<="),
];

const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "break", "class", "continue", "def", "del",
    "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
];

// Longest first so "**=" wins over "**" and "*"
const OPERATORS: &[&str] = &[
    "**=", "//=", ">>=", "<<=", "**", "//", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=",
    "%=", "&=", "|=", "^=", "->", ">>", "<<", "+", "-", "*", "/", "%", "<", ">", "=", "(",
    ")", "[", "]", "{", "}", ",", ":", ".", ";", "&", "|", "^", "~", "@",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Name(Rc<str>),
    Kw(&'static str),
    Op(&'static str),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    /// f-string: raw text between the quotes, and whether it is also raw (rf"...")
    FStr(Rc<str>, bool),
    Newline,
    Indent,
    Dedent,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
}

#[derive(Debug)]
pub struct SyntaxError {
    pub kind: &'static str, // "SyntaxError" or "IndentationError"
    pub msg: String,
    pub line: u32,
}

pub fn err(line: u32, msg: impl Into<String>) -> SyntaxError {
    SyntaxError { kind: "SyntaxError", msg: msg.into(), line }
}

pub fn is_ident_char(c: char) -> bool {
    // Arabic diacritics (harakat, shadda) and tatweel belong to the word
    c == '_' || c.is_alphanumeric() || c == 'ـ'
        || matches!(c as u32, 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06ED)
}

/// Value of an ASCII, Arabic-Indic (٠-٩) or Persian (۰-۹) digit
pub fn digit_value(c: char) -> Option<u32> {
    match c {
        '0'..='9' => Some(c as u32 - '0' as u32),
        '\u{0660}'..='\u{0669}' => Some(c as u32 - 0x0660),
        '\u{06F0}'..='\u{06F9}' => Some(c as u32 - 0x06F0),
        _ => None,
    }
}

pub fn keyword(word: &str) -> Option<&'static str> {
    KEYWORDS.iter().copied().find(|k| *k == word)
}

/// Canonical spelling of a word: Arabic words become their Python meaning
pub fn canonical(word: &str) -> &str {
    ARABIC_WORDS.iter().find(|(ar, _)| *ar == word).map_or(word, |(_, py)| py)
}

pub struct Lexer {
    src: Vec<char>,
    pos: usize,
    line: u32,
    indents: Vec<usize>,
    depth: usize, // bracket nesting: newlines inside brackets are ignored
    out: Vec<Token>,
}

pub fn tokenize(src: &str, first_line: u32) -> Result<Vec<Token>, SyntaxError> {
    let mut lx = Lexer {
        src: src.chars().collect(),
        pos: 0,
        line: first_line,
        indents: vec![0],
        depth: 0,
        out: Vec::new(),
    };
    lx.run()?;
    Ok(lx.out)
}

impl Lexer {
    fn peek(&self, k: usize) -> char {
        self.src.get(self.pos + k).copied().unwrap_or('\0')
    }

    fn push(&mut self, tok: Tok) {
        self.out.push(Token { tok, line: self.line });
    }

    fn run(&mut self) -> Result<(), SyntaxError> {
        let mut at_line_start = true;
        while self.pos < self.src.len() {
            if at_line_start && self.depth == 0 {
                at_line_start = false;
                if self.indentation()? {
                    continue; // blank or comment-only line
                }
            }
            let c = self.peek(0);
            match c {
                '\n' => {
                    self.pos += 1;
                    if self.depth == 0 {
                        if !matches!(self.out.last().map(|t| &t.tok), None | Some(Tok::Newline)) {
                            self.push(Tok::Newline);
                        }
                        at_line_start = true;
                    }
                    self.line += 1;
                }
                ' ' | '\t' | '\r' | '\u{feff}' => self.pos += 1,
                '#' => {
                    while self.pos < self.src.len() && self.peek(0) != '\n' {
                        self.pos += 1;
                    }
                }
                '\\' if self.peek(1) == '\n' || (self.peek(1) == '\r' && self.peek(2) == '\n') => {
                    self.pos += if self.peek(1) == '\r' { 3 } else { 2 };
                    self.line += 1;
                }
                '"' | '\'' => {
                    let s = self.string(false)?;
                    self.push(Tok::Str(s.into()));
                }
                _ if digit_value(c).is_some() || (c == '.' && digit_value(self.peek(1)).is_some()) => self.number()?,
                _ if is_ident_char(c) => self.word()?,
                _ => self.operator()?,
            }
        }
        if !matches!(self.out.last().map(|t| &t.tok), None | Some(Tok::Newline)) {
            self.push(Tok::Newline);
        }
        while self.indents.len() > 1 {
            self.indents.pop();
            self.push(Tok::Dedent);
        }
        self.push(Tok::Eof);
        Ok(())
    }

    /// Measures leading whitespace and emits Indent/Dedent. Returns true for blank lines.
    fn indentation(&mut self) -> Result<bool, SyntaxError> {
        let mut width = 0;
        while matches!(self.peek(0), ' ' | '\t' | '\r' | '\u{feff}') {
            match self.peek(0) {
                ' ' => width += 1,
                '\t' => width = (width / 8 + 1) * 8,
                _ => {}
            }
            self.pos += 1;
        }
        if matches!(self.peek(0), '\n' | '#' | '\0') {
            return Ok(true);
        }
        let current = *self.indents.last().unwrap();
        if width > current {
            self.indents.push(width);
            self.push(Tok::Indent);
        } else {
            while width < *self.indents.last().unwrap() {
                self.indents.pop();
                self.push(Tok::Dedent);
            }
            if width != *self.indents.last().unwrap() {
                return Err(SyntaxError { kind: "IndentationError", msg: "الإزاحة لا تطابق أي مستوى سابق".into(), line: self.line });
            }
        }
        Ok(false)
    }

    fn number(&mut self) -> Result<(), SyntaxError> {
        let mut text = String::new();
        let mut is_float = false;
        loop {
            let c = self.peek(0);
            if let Some(d) = digit_value(c) {
                text.push(char::from_digit(d, 10).unwrap());
            } else if c == '_' && digit_value(self.peek(1)).is_some() {
            } else if (c == '.' || c == '٫') && !is_float && digit_value(self.peek(1)).is_some() {
                text.push('.');
                is_float = true;
            } else if c == '.' && !is_float && !is_ident_char(self.peek(1)) && self.peek(1) != '.' {
                text.push('.'); // "1." is a float
                is_float = true;
            } else if (c == 'e' || c == 'E') && (digit_value(self.peek(1)).is_some()
                || (matches!(self.peek(1), '+' | '-') && digit_value(self.peek(2)).is_some())) {
                text.push('e');
                if matches!(self.peek(1), '+' | '-') {
                    self.pos += 1;
                    text.push(self.peek(0));
                }
                is_float = true;
            } else {
                break;
            }
            self.pos += 1;
        }
        if is_float {
            let v: f64 = text.parse().map_err(|_| err(self.line, format!("رقم غير صالح: {text}")))?;
            self.push(Tok::Float(v));
        } else {
            let v: i64 = text.parse().map_err(|_| err(self.line, format!("الرقم {text} كبير جدا")))?;
            self.push(Tok::Int(v));
        }
        Ok(())
    }

    fn word(&mut self) -> Result<(), SyntaxError> {
        let start = self.pos;
        while is_ident_char(self.peek(0)) {
            self.pos += 1;
        }
        let word: String = self.src[start..self.pos].iter().collect();

        // String prefixes: f"..", r"..", rf".."
        if matches!(self.peek(0), '"' | '\'') {
            let lower = word.to_lowercase();
            if matches!(lower.as_str(), "f" | "r" | "rf" | "fr") {
                let raw = lower.contains('r');
                let s = self.string(raw)?;
                if lower.contains('f') {
                    self.push(Tok::FStr(s.into(), raw));
                } else {
                    self.push(Tok::Str(s.into()));
                }
                return Ok(());
            }
        }

        let word = canonical(&word);
        let tok = if let Some(op) = OPERATORS.iter().find(|o| **o == word) {
            Tok::Op(op) // word operators such as يساوي
        } else if let Some(kw) = keyword(word) {
            Tok::Kw(kw)
        } else {
            Tok::Name(intern(word))
        };
        self.push(tok);
        Ok(())
    }

    fn operator(&mut self) -> Result<(), SyntaxError> {
        let c = self.peek(0);
        // Arabic punctuation
        let mapped = match c {
            '،' => Some(","),
            '؛' => Some(";"),
            _ => None,
        };
        if let Some(op) = mapped {
            self.pos += 1;
            self.push(Tok::Op(op));
            return Ok(());
        }
        for op in OPERATORS {
            if op.chars().enumerate().all(|(i, ch)| self.peek(i) == ch) {
                self.pos += op.chars().count();
                match *op {
                    "(" | "[" | "{" => self.depth += 1,
                    ")" | "]" | "}" => self.depth = self.depth.saturating_sub(1),
                    _ => {}
                }
                self.push(Tok::Op(op));
                return Ok(());
            }
        }
        Err(err(self.line, format!("رمز غير متوقع: '{c}'")))
    }

    /// Reads a string literal. Escapes are processed unless `raw`.
    /// f-strings are returned raw (the parser processes them).
    fn string(&mut self, raw: bool) -> Result<String, SyntaxError> {
        let q = self.peek(0);
        let triple = self.peek(1) == q && self.peek(2) == q;
        let qlen = if triple { 3 } else { 1 };
        self.pos += qlen;
        let start_line = self.line;
        let mut s = String::new();
        loop {
            let c = self.peek(0);
            if self.pos >= self.src.len() || (!triple && c == '\n') {
                return Err(err(start_line, "نص غير مغلق: ينقصه علامة تنصيص"));
            }
            if c == q && (!triple || (self.peek(1) == q && self.peek(2) == q)) {
                self.pos += qlen;
                return Ok(s);
            }
            if c == '\n' {
                self.line += 1;
            }
            if c == '\\' && self.pos + 1 < self.src.len() {
                let n = self.peek(1);
                self.pos += 2;
                if raw {
                    s.push('\\');
                    s.push(n);
                    if n == '\n' {
                        self.line += 1;
                    }
                    continue;
                }
                match n {
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    'r' => s.push('\r'),
                    '0' => s.push('\0'),
                    '\\' => s.push('\\'),
                    '\'' => s.push('\''),
                    '"' => s.push('"'),
                    '\n' => self.line += 1, // line continuation inside a string
                    'x' | 'u' | 'U' => {
                        let len = match n { 'x' => 2, 'u' => 4, _ => 8 };
                        let hex: String = (0..len).map(|i| self.peek(i)).collect();
                        let ch = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                            .ok_or_else(|| err(self.line, "رمز هروب غير صالح في النص"))?;
                        s.push(ch);
                        self.pos += len;
                    }
                    _ => {
                        s.push('\\');
                        s.push(n);
                    }
                }
                continue;
            }
            s.push(c);
            self.pos += 1;
        }
    }
}

/// Processes backslash escapes in an f-string's literal parts
pub fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('\n') => {}
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}
