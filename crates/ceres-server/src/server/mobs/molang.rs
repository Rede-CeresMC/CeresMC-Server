//! A small Molang evaluator for the numeric expressions found in entity files
//! (`"query.last_hit_by_player ? Math.Random(1,3) : 0"`, `"query.is_baby ? 0.02 : 0.001"`).
//! Unknown queries and variables read as 0.

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f32),
    Ident(String),
    Str(String),
    Op(&'static str),
}

fn lex(s: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() || ch == ';' {
            i += 1;
        } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let st = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
                i += 1;
            }
            if i < c.len() && c[i] == 'f' {
                i += 1;
            }
            let txt: String = c[st..i].iter().filter(|x| **x != 'f').collect();
            out.push(Tok::Num(txt.parse().unwrap_or(0.0)));
        } else if ch.is_alphabetic() || ch == '_' {
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '.') {
                i += 1;
            }
            out.push(Tok::Ident(c[st..i].iter().collect::<String>().to_ascii_lowercase()));
        } else if ch == '\'' {
            let st = i + 1;
            i += 1;
            while i < c.len() && c[i] != '\'' {
                i += 1;
            }
            out.push(Tok::Str(c[st..i.min(c.len())].iter().collect()));
            i += 1;
        } else {
            let two: String = c[i..(i + 2).min(c.len())].iter().collect();
            let op = match two.as_str() {
                "==" => Some("=="),
                "!=" => Some("!="),
                "<=" => Some("<="),
                ">=" => Some(">="),
                "&&" => Some("&&"),
                "||" => Some("||"),
                "??" => Some("??"),
                _ => None,
            };
            if let Some(op) = op {
                out.push(Tok::Op(op));
                i += 2;
                continue;
            }
            let op = match ch {
                '+' => "+",
                '-' => "-",
                '*' => "*",
                '/' => "/",
                '<' => "<",
                '>' => ">",
                '!' => "!",
                '?' => "?",
                ':' => ":",
                '(' => "(",
                ')' => ")",
                ',' => ",",
                _ => {
                    i += 1;
                    continue;
                }
            };
            out.push(Tok::Op(op));
            i += 1;
        }
    }
    out
}

struct P<'a, Q: Fn(&str, &[f32]) -> f32> {
    t: Vec<Tok>,
    i: usize,
    q: &'a Q,
}

impl<Q: Fn(&str, &[f32]) -> f32> P<'_, Q> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn eat(&mut self, op: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Op(o)) if *o == op) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> f32 {
        let c = self.or();
        if self.eat("?") {
            let a = self.expr();
            let b = if self.eat(":") { self.expr() } else { 0.0 };
            return if c != 0.0 { a } else { b };
        }
        if self.eat("??") {
            let b = self.expr();
            return if c != 0.0 { c } else { b };
        }
        c
    }
    fn or(&mut self) -> f32 {
        let mut a = self.and();
        while self.eat("||") {
            let b = self.and();
            a = ((a != 0.0) || (b != 0.0)) as i32 as f32;
        }
        a
    }
    fn and(&mut self) -> f32 {
        let mut a = self.cmp();
        while self.eat("&&") {
            let b = self.cmp();
            a = ((a != 0.0) && (b != 0.0)) as i32 as f32;
        }
        a
    }
    fn cmp(&mut self) -> f32 {
        let a = self.add();
        for op in ["==", "!=", "<=", ">=", "<", ">"] {
            if self.eat(op) {
                let b = self.add();
                let r = match op {
                    "==" => (a - b).abs() < 1e-6,
                    "!=" => (a - b).abs() >= 1e-6,
                    "<=" => a <= b,
                    ">=" => a >= b,
                    "<" => a < b,
                    _ => a > b,
                };
                return r as i32 as f32;
            }
        }
        a
    }
    fn add(&mut self) -> f32 {
        let mut a = self.mul();
        loop {
            if self.eat("+") {
                a += self.mul();
            } else if self.eat("-") {
                a -= self.mul();
            } else {
                return a;
            }
        }
    }
    fn mul(&mut self) -> f32 {
        let mut a = self.unary();
        loop {
            if self.eat("*") {
                a *= self.unary();
            } else if self.eat("/") {
                let b = self.unary();
                a = if b == 0.0 { 0.0 } else { a / b };
            } else {
                return a;
            }
        }
    }
    fn unary(&mut self) -> f32 {
        if self.eat("-") {
            return -self.unary();
        }
        if self.eat("!") {
            return (self.unary() == 0.0) as i32 as f32;
        }
        self.atom()
    }
    fn atom(&mut self) -> f32 {
        match self.t.get(self.i).cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                n
            }
            Some(Tok::Str(_)) => {
                self.i += 1;
                0.0
            }
            Some(Tok::Op("(")) => {
                self.i += 1;
                let v = self.expr();
                self.eat(")");
                v
            }
            Some(Tok::Ident(name)) => {
                self.i += 1;
                let mut args = Vec::new();
                if self.eat("(") {
                    if !self.eat(")") {
                        loop {
                            args.push(self.expr());
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.eat(")");
                    }
                }
                call(&name, &args).unwrap_or_else(|| match name.as_str() {
                    "true" => 1.0,
                    "false" => 0.0,
                    _ => (self.q)(&name, &args),
                })
            }
            _ => {
                self.i += 1;
                0.0
            }
        }
    }
}

fn call(name: &str, a: &[f32]) -> Option<f32> {
    let g = |i: usize| a.get(i).copied().unwrap_or(0.0);
    Some(match name {
        "math.random" => super::json::rand_f(g(0), g(1)),
        "math.random_integer" => super::json::rand_i(g(0).round() as i32, g(1).round() as i32) as f32,
        "math.die_roll" => (0..g(0).max(0.0) as i32).map(|_| super::json::rand_f(g(1), g(2))).sum(),
        "math.die_roll_integer" => (0..g(0).max(0.0) as i32).map(|_| super::json::rand_i(g(1) as i32, g(2) as i32) as f32).sum(),
        "math.min" => g(0).min(g(1)),
        "math.max" => g(0).max(g(1)),
        "math.abs" => g(0).abs(),
        "math.floor" => g(0).floor(),
        "math.ceil" => g(0).ceil(),
        "math.round" => g(0).round(),
        "math.trunc" => g(0).trunc(),
        "math.sqrt" => g(0).max(0.0).sqrt(),
        "math.pow" => g(0).powf(g(1)),
        "math.mod" => {
            if g(1) == 0.0 {
                0.0
            } else {
                g(0) % g(1)
            }
        }
        "math.clamp" => g(0).clamp(g(1).min(g(2)), g(2).max(g(1))),
        "math.lerp" => g(0) + (g(1) - g(0)) * g(2),
        "math.sin" => g(0).to_radians().sin(),
        "math.cos" => g(0).to_radians().cos(),
        "math.pi" => std::f32::consts::PI,
        _ => return None,
    })
}

/// Evaluates `expr`; `query(name, args)` answers `query.*`/`variable.*` names (lowercase).
pub(crate) fn eval(expr: &str, query: &impl Fn(&str, &[f32]) -> f32) -> f32 {
    let mut p = P { t: lex(expr), i: 0, q: query };
    let v = p.expr();
    if v.is_finite() { v } else { 0.0 }
}

/// A JSON value that is a number or a Molang string.
pub(crate) fn value(v: Option<&serde_json::Value>, default: f32, query: &impl Fn(&str, &[f32]) -> f32) -> f32 {
    match v {
        Some(serde_json::Value::String(s)) => eval(s, query),
        other => super::json::as_num(other).unwrap_or(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions() {
        let q = |n: &str, _: &[f32]| if n == "query.is_baby" { 1.0 } else { 0.0 };
        assert_eq!(eval("query.is_baby ? 0.02 : 0.001", &q), 0.02);
        assert_eq!(eval("query.last_hit_by_player ? 5 : 0", &q), 0.0);
        assert_eq!(eval("1 + 2 * 3", &q), 7.0);
        assert_eq!(eval("(1 + 2) * 3", &q), 9.0);
        assert_eq!(eval("!query.is_baby", &q), 0.0);
        let r = eval("Math.Random(1,3)", &q);
        assert!((1.0..=3.0).contains(&r));
        let r = eval("math.random_integer(2, 4)", &q);
        assert!([2.0, 3.0, 4.0].contains(&r));
    }
}
