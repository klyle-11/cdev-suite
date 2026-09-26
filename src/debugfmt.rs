//! Parse Rust `{:?}` output back into JSON, so the Rust SDK needs no serde
//! and still gets type shapes and data-structure diagrams.
//!
//!   Node { val: 1, next: Some(Node { val: 2, next: None }) }
//!     → {"__class":"Node","val":1,"next":{"__class":"Node","val":2,"next":null}}
//!
//! `Some(x)` → x, `None` → null, `Box`/`Rc` are already transparent in Debug,
//! `RefCell { value: x }` → x, `{k: v}` → object (string keys) or Map, `{a, b}` → Set.

use serde_json::{json, Map, Value};

pub fn parse(s: &str) -> Option<Value> {
    let mut p = P { s: s.as_bytes(), i: 0, depth: 0 };
    let v = p.value()?;
    p.ws();
    if p.i == p.s.len() { Some(v) } else { None }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl<'a> P<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\t' | b'\r')) {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.depth += 1;
        if self.depth > 200 {
            return None;
        }
        self.ws();
        let v = match self.peek()? {
            b'"' => Value::String(self.string(b'"')?),
            b'\'' => Value::String(self.string(b'\'')?),
            b'[' => {
                self.i += 1;
                Value::Array(self.list(b']')?)
            }
            b'(' => {
                self.i += 1;
                Value::Array(self.list(b')')?)
            }
            b'{' => self.braces()?,
            c if c == b'-' || c.is_ascii_digit() => self.number()?,
            c if c.is_ascii_alphabetic() || c == b'_' => self.named()?,
            _ => return None,
        };
        self.depth -= 1;
        Some(v)
    }

    fn string(&mut self, q: u8) -> Option<String> {
        self.i += 1;
        let mut out = String::new();
        let start = self.i;
        let mut buf: Vec<u8> = Vec::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                b'\\' => {
                    let e = self.peek()?;
                    self.i += 1;
                    match e {
                        b'n' => buf.push(b'\n'),
                        b't' => buf.push(b'\t'),
                        b'r' => buf.push(b'\r'),
                        b'0' => buf.push(0),
                        b'u' => {
                            // \u{1f600}
                            if self.eat(b'{') {
                                let hs = self.i;
                                while self.peek()? != b'}' {
                                    self.i += 1;
                                }
                                let hex = std::str::from_utf8(&self.s[hs..self.i]).ok()?;
                                self.i += 1;
                                let ch = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
                                let mut b = [0u8; 4];
                                buf.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                            }
                        }
                        other => buf.push(other),
                    }
                }
                c if c == q => {
                    out.push_str(&String::from_utf8_lossy(&buf));
                    let _ = start;
                    return Some(out);
                }
                c => buf.push(c),
            }
        }
        None
    }

    fn number(&mut self) -> Option<Value> {
        let st = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'+' | b'.' | b'_')) {
            self.i += 1;
        }
        let t = std::str::from_utf8(&self.s[st..self.i]).ok()?.replace('_', "");
        if let Ok(n) = t.parse::<i64>() {
            return Some(json!(n));
        }
        if let Ok(n) = t.parse::<u64>() {
            return Some(json!(n));
        }
        match t.parse::<f64>() {
            Ok(f) if f.is_finite() => Some(json!(f)),
            Ok(f) => Some(json!({"__t": "f64", "v": f.to_string()})),
            Err(_) => Some(Value::String(t)),
        }
    }

    fn ident(&mut self) -> String {
        // identifiers, plus `::` path separators (a single `:` ends a field name)
        let st = self.i;
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_alphanumeric() || c == b'_' => self.i += 1,
                Some(b':') if self.s.get(self.i + 1) == Some(&b':') => self.i += 2,
                _ => break,
            }
        }
        String::from_utf8_lossy(&self.s[st..self.i]).into_owned()
    }

    fn named(&mut self) -> Option<Value> {
        let name = self.ident();
        match name.as_str() {
            "None" => return Some(Value::Null),
            "true" => return Some(Value::Bool(true)),
            "false" => return Some(Value::Bool(false)),
            "NaN" | "inf" => return Some(json!({"__t": "f64", "v": name})),
            _ => {}
        }
        let short = name.rsplit("::").next().unwrap_or(&name).to_string();
        self.ws();
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let mut items = self.list(b')')?;
                if items.len() == 1 && matches!(short.as_str(), "Some" | "Box" | "Rc" | "Arc" | "Cell" | "Wrapping" | "Reverse") {
                    return items.pop();
                }
                let mut o = Map::new();
                o.insert("__class".into(), Value::String(short));
                for (i, v) in items.into_iter().enumerate() {
                    o.insert(i.to_string(), v);
                }
                Some(Value::Object(o))
            }
            Some(b'{') => {
                self.i += 1;
                let mut o = Map::new();
                loop {
                    if self.eat(b'}') {
                        break;
                    }
                    self.ws();
                    if self.s[self.i..].starts_with(b"..") {
                        self.i += 2;
                        continue;
                    }
                    let key = self.ident();
                    if key.is_empty() || !self.eat(b':') {
                        return None;
                    }
                    let v = self.value()?;
                    o.insert(key, v);
                    self.eat(b',');
                }
                // RefCell { value: x } / Mutex { data: x, .. } → x
                if matches!(short.as_str(), "RefCell" | "Mutex" | "RwLock") {
                    if let Some(v) = o.remove("value").or_else(|| o.remove("data")) {
                        return Some(v);
                    }
                }
                let mut out = Map::new();
                out.insert("__class".into(), Value::String(short));
                out.extend(o);
                Some(Value::Object(out))
            }
            _ => Some(Value::String(name)), // unit struct / enum variant
        }
    }

    fn list(&mut self, close: u8) -> Option<Vec<Value>> {
        let mut out = Vec::new();
        loop {
            if self.eat(close) {
                return Some(out);
            }
            out.push(self.value()?);
            if !self.eat(b',') {
                return if self.eat(close) { Some(out) } else { None };
            }
        }
    }

    /// `{k: v, …}` (map) or `{a, b}` (set)
    fn braces(&mut self) -> Option<Value> {
        self.i += 1;
        let mut entries: Vec<(Value, Value)> = Vec::new();
        let mut set: Vec<Value> = Vec::new();
        loop {
            if self.eat(b'}') {
                break;
            }
            let k = self.value()?;
            if self.eat(b':') {
                let v = self.value()?;
                entries.push((k, v));
            } else {
                set.push(k);
            }
            if !self.eat(b',') {
                if !self.eat(b'}') {
                    return None;
                }
                break;
            }
        }
        if !set.is_empty() {
            return Some(json!({"__t": "Set", "size": set.len(), "values": set}));
        }
        if entries.iter().all(|(k, _)| k.is_string()) {
            let mut o = Map::new();
            for (k, v) in entries {
                o.insert(k.as_str().unwrap().to_string(), v);
            }
            return Some(Value::Object(o));
        }
        let n = entries.len();
        let arr: Vec<Value> = entries.into_iter().map(|(k, v)| json!([k, v])).collect();
        Some(json!({"__t": "Map", "size": n, "entries": arr}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_debug() {
        assert_eq!(parse("[1, 2, 3]").unwrap(), json!([1, 2, 3]));
        assert_eq!(parse(r#"Some("hi")"#).unwrap(), json!("hi"));
        assert_eq!(
            parse("ListNode { val: 1, next: Some(ListNode { val: 2, next: None }) }").unwrap(),
            json!({"__class": "ListNode", "val": 1, "next": {"__class": "ListNode", "val": 2, "next": null}})
        );
        assert_eq!(parse(r#"{"a": 1, "b": 2}"#).unwrap(), json!({"a": 1, "b": 2}));
        assert_eq!(parse("{1: [2, 3], 2: []}").unwrap()["__t"], "Map");
        assert_eq!(parse("{3, 1}").unwrap()["__t"], "Set");
        assert_eq!(parse("RefCell { value: [1] }").unwrap(), json!([1]));
        assert_eq!(parse("(1, 'x', -2.5)").unwrap(), json!([1, "x", -2.5]));
        assert_eq!(parse("Point(3, 4)").unwrap(), json!({"__class": "Point", "0": 3, "1": 4}));
    }
}
