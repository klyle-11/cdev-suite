//! Static view of a JS/TS file: what can be said about the code by reading
//! it, without running it.
//!
//! For every named function (and the top level) the report lists the
//! signature, parameters and locals with their declared or literal-inferred
//! types, which functions it calls and is called by, what it changes outside
//! itself, and the shape of its control flow. Anonymous callbacks are folded
//! into the function that contains them.
//!
//! Everything here is derived from the syntax tree alone. Values are never
//! known; calls through an object (`obj.method()`) are not resolved to a
//! class, and mutation through a method is recognised by name (`push`, `set`…).

use crate::instrument::line_starts;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::operator::{BinaryOperator, UnaryOperator};
use oxc_syntax::scope::ScopeFlags;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

pub const TOP: &str = "(top level)";
const EXTS: [&str; 8] = ["js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx"];
/// Methods that change the object they are called on.
const MUTATORS: [&str; 14] = ["push", "pop", "shift", "unshift", "splice", "sort", "reverse", "fill", "copyWithin", "set", "add", "delete", "clear", "append"];

#[derive(Serialize, Default, Clone)]
pub struct Report {
    pub file: String,
    pub lang: &'static str,
    pub lines: usize,
    pub imports: Vec<Import>,
    pub exports: Vec<String>,
    pub types: Vec<TypeDecl>,
    pub fns: Vec<Func>,
}

#[derive(Serialize, Clone)]
pub struct Import {
    pub from: String,
    pub names: Vec<String>,
    pub line: usize,
}

#[derive(Serialize, Clone)]
pub struct TypeDecl {
    pub name: String,
    pub kind: &'static str,
    pub line: usize,
}

#[derive(Serialize, Clone)]
pub struct Var {
    pub name: String,
    pub kind: &'static str,
    pub line: usize,
    /// declared type, or the type of a literal initializer
    pub ty: Option<String>,
    /// where the value comes from (initializer text, `each of xs`, `part of obj`)
    pub init: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct Call {
    pub name: String,
    pub lines: Vec<usize>,
    /// the function in this file it resolves to
    pub target: Option<String>,
}

#[derive(Serialize, Default, Clone)]
pub struct Func {
    pub name: String,
    pub kind: &'static str,
    pub signature: String,
    pub line: usize,
    pub end: usize,
    /// nesting inside other functions (0 = declared at the top level)
    pub level: usize,
    #[serde(rename = "async")]
    pub is_async: bool,
    pub generator: bool,
    pub exported: bool,
    pub recursive: bool,
    pub params: Vec<Var>,
    pub ret: Option<String>,
    pub locals: Vec<Var>,
    pub calls: Vec<Call>,
    pub called_by: Vec<String>,
    /// parameters (or `this`) whose contents are changed
    pub mutates: Vec<String>,
    pub writes_outer: Vec<String>,
    pub reads_outer: Vec<String>,
    pub loops: u32,
    pub branches: u32,
    pub returns: u32,
    pub return_values: Vec<String>,
    pub throws: u32,
    pub awaits: u32,
    /// deepest nesting of loops and branches
    pub depth: u32,
    pub tags: Vec<String>,
    /// one plain sentence built from the facts above
    pub summary: String,
    /// short form for a CodeLens
    pub lens: String,
    /// the detail lines, ready to print: (label, text)
    pub rows: Vec<(&'static str, String)>,
}

pub fn supported(file: &str) -> bool {
    ext(file).map_or(false, |e| EXTS.contains(&e.as_str()))
}

fn ext(file: &str) -> Option<String> {
    std::path::Path::new(file).extension().map(|e| e.to_string_lossy().to_lowercase())
}

/// Analyse `src`; `file` picks the dialect and labels the report.
pub fn explain(src: &str, file: &str) -> Result<Report, String> {
    if !supported(file) {
        return Err(format!("the static view reads JS/TS for now ({})", EXTS.map(|e| format!(".{e}")).join(" ")));
    }
    let alloc = Allocator::default();
    let st = SourceType::from_path(file).unwrap_or_else(|_| SourceType::mjs());
    let ret = Parser::new(&alloc, src, st).parse();
    if ret.panicked || !ret.errors.is_empty() {
        return Err(ret.errors.first().map(|e| e.to_string()).unwrap_or_else(|| "parse failed".into()));
    }
    let program = &ret.program;
    let lines = line_starts(src);
    let total = src.lines().count();
    let mut a = Analyzer {
        src,
        lines,
        fns: vec![Func { name: TOP.into(), kind: "module", signature: TOP.into(), line: 1, end: total.max(1), ..Func::default() }],
        scopes: Vec::new(),
        pending: None,
        kind: None,
        classes: Vec::new(),
        iter_src: None,
        imports: Vec::new(),
        exports: Vec::new(),
        exported: HashSet::new(),
        types: Vec::new(),
        referenced: HashSet::new(),
    };
    let mut d = Decls::default();
    for s in &program.body {
        d.visit_statement(s);
    }
    a.scopes.push(Scope { f: 0, own: true, vars: d.vars, params: HashSet::new(), other: d.other, depth: 0 });
    a.visit_program(program);

    let is_ts = matches!(ext(file).as_deref(), Some("ts" | "mts" | "cts" | "tsx"));
    let mut r = Report {
        file: file.to_string(),
        lang: if is_ts { "typescript" } else { "javascript" },
        lines: total,
        imports: a.imports,
        exports: a.exports,
        types: a.types,
        fns: a.fns,
    };
    finish(&mut r, &a.exported, &a.referenced);
    Ok(r)
}

/// Cross-reference calls, then derive tags, the summary and the printable rows.
fn finish(r: &mut Report, exported: &HashSet<String>, referenced: &HashSet<String>) {
    let index: HashMap<String, usize> = r.fns.iter().enumerate().rev().map(|(i, f)| (f.name.clone(), i)).collect();
    let mut called_by: Vec<Vec<String>> = vec![Vec::new(); r.fns.len()];
    for i in 0..r.fns.len() {
        let me = r.fns[i].name.clone();
        let class = me.rsplit_once('.').map(|(c, _)| c.to_string());
        for c in &mut r.fns[i].calls {
            let target = if let Some(m) = c.name.strip_prefix("this.") {
                class.as_ref().map(|k| format!("{k}.{m}"))
            } else if let Some(k) = c.name.strip_prefix("new ") {
                Some(format!("{k}.constructor"))
            } else {
                Some(c.name.clone())
            };
            c.target = target.filter(|t| index.contains_key(t));
            if let Some(t) = &c.target {
                let list = &mut called_by[index[t]];
                if !list.contains(&me) {
                    list.push(me.clone());
                }
            }
        }
    }
    for (i, f) in r.fns.iter_mut().enumerate() {
        f.called_by = std::mem::take(&mut called_by[i]);
        f.recursive = f.called_by.contains(&f.name);
        f.exported = exported.contains(&f.name) || r.exports.contains(&f.name);
        let top = f.kind == "module";
        let mut tags: Vec<String> = Vec::new();
        if f.is_async {
            tags.push("async".into());
        }
        if f.generator {
            tags.push("generator".into());
        }
        if f.exported {
            tags.push("exported".into());
        }
        if f.recursive {
            tags.push("recursive".into());
        }
        let inputs: Vec<&String> = f.mutates.iter().filter(|m| *m != "this").collect();
        if !inputs.is_empty() {
            tags.push("changes its input".into());
        }
        if !top && !f.writes_outer.is_empty() {
            tags.push("changes outer state".into());
        }
        // touches only its own parameters and locals, and calls nothing but itself and methods on them
        let outside = f.calls.iter().any(|c| c.target.as_ref() != Some(&f.name) && !own_call(f, &c.name));
        if !top && f.mutates.is_empty() && f.writes_outer.is_empty() && f.reads_outer.is_empty() && !outside && f.awaits == 0 {
            tags.push("self-contained".into());
        }
        if matches!(f.kind, "function" | "arrow") && f.level == 0 && !f.exported && f.called_by.is_empty() && !referenced.contains(&f.name) {
            tags.push("not used in this file".into());
        }
        f.tags = tags;
        f.summary = summary(f);
        f.lens = lens(f);
        f.rows = rows(f);
    }
}

/// A call that stays inside the function's own data: a method on a parameter or local, or a known value-only builtin.
fn own_call(f: &Func, name: &str) -> bool {
    let root = name.trim_start_matches("new ").split(['.', '[']).next().unwrap_or("");
    f.params.iter().chain(&f.locals).any(|v| v.name == root)
        || matches!(root, "Math" | "Number" | "String" | "Boolean" | "Array" | "Object" | "JSON" | "parseInt" | "parseFloat" | "isNaN" | "isFinite" | "Map" | "Set")
}

fn plural(n: u32, word: &str) -> String {
    let es = word.ends_with("ch");
    format!("{n} {word}{}", if n == 1 { "" } else if es { "es" } else { "s" })
}

fn summary(f: &Func) -> String {
    let mut s: Vec<String> = Vec::new();
    if f.kind == "module" {
        s.push("The file's top-level code and its inline callbacks".into());
    } else if f.params.is_empty() {
        s.push("Takes nothing".into());
    } else {
        s.push(format!("Takes {}", f.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")));
    }
    if f.loops > 0 {
        s.push(format!("Loops ({})", plural(f.loops, "loop")));
    }
    let local: Vec<&str> = f.calls.iter().filter_map(|c| c.target.as_deref()).filter(|t| *t != f.name).collect();
    let other = f.calls.iter().filter(|c| c.target.is_none()).count();
    match (local.is_empty(), other) {
        (false, 0) => s.push(format!("Calls {}", local.join(", "))),
        (false, n) => s.push(format!("Calls {} and {n} outside the file", local.join(", "))),
        (true, n) if n > 0 => s.push(format!("Calls {n} outside the file")),
        _ => {}
    }
    if f.recursive {
        s.push("Calls itself".into());
    }
    if f.awaits > 0 {
        s.push(format!("Waits on {}", plural(f.awaits, "async result")));
    }
    if !f.mutates.is_empty() {
        s.push(format!("Changes {}", f.mutates.join(", ")));
    }
    if f.kind != "module" {
        if !f.writes_outer.is_empty() {
            s.push(format!("Writes outer {}", f.writes_outer.join(", ")));
        }
        if !f.reads_outer.is_empty() {
            s.push(format!("Reads outer {}", f.reads_outer.join(", ")));
        }
    }
    if f.throws > 0 {
        s.push("Can throw".into());
    }
    if f.kind != "module" && f.kind != "constructor" {
        s.push(match (&f.ret, f.return_values.is_empty()) {
            (Some(t), _) => format!("Returns {t}"),
            (None, false) => format!("Returns {}", f.return_values.join(" or ")),
            (None, true) => "Returns nothing".into(),
        });
    }
    s.join(". ") + "."
}

fn lens(f: &Func) -> String {
    let mut s: Vec<String> = Vec::new();
    if !f.calls.is_empty() {
        s.push(format!("calls {}", f.calls.len()));
    }
    if !f.called_by.is_empty() {
        s.push(format!("called by {}", f.called_by.join(", ")));
    }
    if f.loops > 0 {
        s.push(plural(f.loops, "loop"));
    }
    s.extend(f.tags.iter().filter(|t| !matches!(t.as_str(), "async" | "generator" | "exported")).cloned());
    if s.is_empty() { "no calls".into() } else { s.join(" · ") }
}

fn var_text(v: &Var) -> String {
    let mut s = v.name.clone();
    if let Some(t) = &v.ty {
        s.push_str(&format!(": {t}"));
    }
    if let Some(i) = &v.init {
        s.push_str(&format!(" = {i}"));
    }
    s
}

fn rows(f: &Func) -> Vec<(&'static str, String)> {
    let mut r: Vec<(&'static str, String)> = Vec::new();
    let vars = |v: &[Var]| v.iter().map(var_text).collect::<Vec<_>>().join("  ·  ");
    if !f.tags.is_empty() {
        r.push(("tags", f.tags.join(" · ")));
    }
    if !f.params.is_empty() {
        r.push(("params", vars(&f.params)));
    }
    if !f.locals.is_empty() {
        r.push((if f.kind == "module" { "names" } else { "locals" }, vars(&f.locals)));
    }
    if !f.calls.is_empty() {
        let c: Vec<String> = f.calls.iter().map(|c| {
            let at = if c.lines.len() > 1 { format!(":{} ×{}", c.lines[0], c.lines.len()) } else { format!(":{}", c.lines[0]) };
            match &c.target {
                Some(t) if *t != c.name => format!("{}{at} → {t}", c.name),
                Some(_) => format!("{}{at} (here)", c.name),
                None => format!("{}{at}", c.name),
            }
        }).collect();
        r.push(("calls", c.join("  ·  ")));
    }
    if !f.called_by.is_empty() {
        r.push(("called by", f.called_by.join(", ")));
    }
    if !f.mutates.is_empty() {
        r.push(("changes", f.mutates.join(", ")));
    }
    if !f.writes_outer.is_empty() {
        r.push(("writes outer", f.writes_outer.join(", ")));
    }
    if !f.reads_outer.is_empty() {
        r.push(("reads outer", f.reads_outer.join(", ")));
    }
    if !f.return_values.is_empty() || f.ret.is_some() {
        let vals = f.return_values.join("  ·  ");
        r.push(("returns", match &f.ret {
            Some(t) if vals.is_empty() => t.clone(),
            Some(t) => format!("{t}  ←  {vals}"),
            None => vals,
        }));
    }
    let mut flow: Vec<String> = Vec::new();
    for (n, w) in [(f.loops, "loop"), (f.branches, "branch"), (f.returns, "return"), (f.throws, "throw"), (f.awaits, "await")] {
        if n > 0 {
            flow.push(plural(n, w));
        }
    }
    if f.depth > 0 {
        flow.push(format!("nesting {}", f.depth));
    }
    if !flow.is_empty() {
        r.push(("flow", flow.join(" · ")));
    }
    r
}

/// The whole report as plain text (`cdev explain --text`).
pub fn text(r: &Report) -> String {
    let mut o = format!("{} · {} · {} lines · {}\n", r.file, r.lang, r.lines, plural(r.fns.len() as u32 - 1, "function"));
    for i in &r.imports {
        o.push_str(&format!("  import     {} ← \"{}\"  :{}\n", if i.names.is_empty() { "(side effects)".to_string() } else { i.names.join(", ") }, i.from, i.line));
    }
    if !r.exports.is_empty() {
        o.push_str(&format!("  exports    {}\n", r.exports.join(", ")));
    }
    if !r.types.is_empty() {
        o.push_str(&format!("  types      {}\n", r.types.iter().map(|t| format!("{} {} :{}", t.kind, t.name, t.line)).collect::<Vec<_>>().join("  ·  ")));
    }
    for f in &r.fns {
        let pad = "  ".repeat(f.level);
        o.push_str(&format!("\n{pad}{}   lines {}–{}\n", f.signature, f.line, f.end));
        o.push_str(&format!("{pad}  {}\n", f.summary));
        for (label, body) in &f.rows {
            o.push_str(&format!("{pad}  {label:<13}{body}\n"));
        }
    }
    o
}

// ---------------------------------------------------------------- declarations pre-pass

/// Names declared directly in a function body (not inside nested functions),
/// so a reference can be resolved before its declaration has been walked.
#[derive(Default)]
struct Decls {
    vars: HashSet<String>,
    /// functions, classes and imports: named things that are not data
    other: HashSet<String>,
}

fn is_fn_expr(e: &Expression) -> bool {
    matches!(e, Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_))
}

impl<'a> Visit<'a> for Decls {
    fn visit_function(&mut self, it: &Function<'a>, _flags: ScopeFlags) {
        if let Some(id) = &it.id {
            self.other.insert(id.name.to_string());
        }
    }

    fn visit_arrow_function_expression(&mut self, _it: &ArrowFunctionExpression<'a>) {}

    fn visit_class(&mut self, it: &Class<'a>) {
        if let Some(id) = &it.id {
            self.other.insert(id.name.to_string());
        }
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        let is_fn = it.init.as_ref().map_or(false, is_fn_expr);
        for id in it.id.get_binding_identifiers() {
            if is_fn { self.other.insert(id.name.to_string()) } else { self.vars.insert(id.name.to_string()) };
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_catch_parameter(&mut self, it: &CatchParameter<'a>) {
        for id in it.pattern.get_binding_identifiers() {
            self.vars.insert(id.name.to_string());
        }
    }

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        for s in it.specifiers.iter().flatten() {
            self.other.insert(s.name().to_string());
        }
    }
}

// ---------------------------------------------------------------- analysis

struct Scope {
    /// index of the function this scope reports into
    f: usize,
    /// false for an anonymous callback folded into its enclosing function
    own: bool,
    vars: HashSet<String>,
    params: HashSet<String>,
    other: HashSet<String>,
    depth: u32,
}

#[derive(PartialEq)]
enum Where {
    Local,
    Param,
    OuterVar,
    Other,
    Global,
}

struct Analyzer<'s> {
    src: &'s str,
    lines: Vec<u32>,
    fns: Vec<Func>,
    scopes: Vec<Scope>,
    /// name for the next function literal (from `const f = …`, methods, object keys)
    pending: Option<String>,
    kind: Option<&'static str>,
    classes: Vec<String>,
    /// `each of xs` for the loop variable of the for-in/of being entered
    iter_src: Option<String>,
    imports: Vec<Import>,
    exports: Vec<String>,
    exported: HashSet<String>,
    types: Vec<TypeDecl>,
    referenced: HashSet<String>,
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    if !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

/// The name at the start of a member chain: `a` in `a.b[c].d`, `this` in `this.x`.
fn root_of<'e>(mut e: &'e Expression<'e>) -> Option<&'e str> {
    loop {
        e = e.get_inner_expression();
        match e {
            Expression::Identifier(id) => return Some(id.name.as_str()),
            Expression::ThisExpression(_) => return Some("this"),
            Expression::StaticMemberExpression(m) => e = &m.object,
            Expression::ComputedMemberExpression(m) => e = &m.object,
            Expression::PrivateFieldExpression(m) => e = &m.object,
            _ => return None,
        }
    }
}

impl<'s> Analyzer<'s> {
    fn line(&self, pos: u32) -> usize {
        self.lines.partition_point(|&s| s <= pos)
    }

    /// Source text of a span on one line, shortened to `max` characters.
    fn text(&self, span: Span, max: usize) -> String {
        let s = self.src[span.start as usize..span.end as usize].split_whitespace().collect::<Vec<_>>().join(" ");
        crate::event::truncate(&s, max)
    }

    fn cur(&self) -> usize {
        self.scopes.last().map_or(0, |s| s.f)
    }

    fn f(&mut self) -> &mut Func {
        let i = self.cur();
        &mut self.fns[i]
    }

    fn own(&self) -> bool {
        self.scopes.last().map_or(true, |s| s.own)
    }

    fn resolve(&self, name: &str) -> Where {
        let cur = self.cur();
        for s in self.scopes.iter().rev() {
            let local = s.f == cur;
            if s.params.contains(name) {
                return if local { Where::Param } else { Where::OuterVar };
            }
            if s.vars.contains(name) {
                return if local { Where::Local } else { Where::OuterVar };
            }
            if s.other.contains(name) {
                return Where::Other;
            }
        }
        Where::Global
    }

    /// `name` is assigned (`member`: something inside it is). `by_name`: inferred from a method name only.
    fn write(&mut self, name: &str, member: bool, by_name: bool) {
        if name == "this" {
            if member && self.fns[self.cur()].kind != "constructor" {
                push_unique(&mut self.f().mutates, "this");
            }
            return;
        }
        if matches!(name, "module" | "exports") {
            return;
        }
        match self.resolve(name) {
            Where::Param if member => push_unique(&mut self.f().mutates, name),
            Where::OuterVar => push_unique(&mut self.f().writes_outer, name),
            Where::Global if !by_name => push_unique(&mut self.f().writes_outer, name),
            _ => {}
        }
    }

    fn nest(&mut self, d: i32) {
        if let Some(s) = self.scopes.last_mut() {
            s.depth = (s.depth as i32 + d).max(0) as u32;
            let depth = s.depth;
            let f = self.f();
            f.depth = f.depth.max(depth);
        }
    }

    /// Literal type of an expression, when the syntax alone decides it.
    fn infer(&self, e: &Expression) -> Option<String> {
        Some(match e.without_parentheses() {
            Expression::NumericLiteral(_) => "number".into(),
            Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => "string".into(),
            Expression::BooleanLiteral(_) => "boolean".into(),
            Expression::NullLiteral(_) => "null".into(),
            Expression::BigIntLiteral(_) => "bigint".into(),
            Expression::RegExpLiteral(_) => "RegExp".into(),
            Expression::Identifier(id) if id.name == "undefined" => "undefined".into(),
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => "function".into(),
            Expression::TSAsExpression(t) => self.text(t.type_annotation.span(), 40),
            Expression::NewExpression(n) => match &n.callee {
                Expression::Identifier(id) => id.name.to_string(),
                _ => return None,
            },
            Expression::ArrayExpression(a) => {
                let kinds: Vec<Option<String>> = a.elements.iter().map(|el| el.as_expression().and_then(|x| self.infer(x))).collect();
                match kinds.first() {
                    None => "[]".into(),
                    Some(Some(first)) if kinds.iter().all(|k| k.as_ref() == Some(first)) => {
                        if first.contains(' ') { format!("({first})[]") } else { format!("{first}[]") }
                    }
                    _ => "array".into(),
                }
            }
            Expression::ObjectExpression(o) => {
                let keys: Vec<String> = o.properties.iter().filter_map(|p| match p {
                    ObjectPropertyKind::ObjectProperty(p) => p.key.static_name().map(|n| n.to_string()),
                    ObjectPropertyKind::SpreadProperty(_) => Some("…".into()),
                }).collect();
                if keys.is_empty() {
                    "{}".into()
                } else if keys.len() > 5 {
                    format!("{{ {}, … }}", keys[..5].join(", "))
                } else {
                    format!("{{ {} }}", keys.join(", "))
                }
            }
            Expression::UnaryExpression(u) => match u.operator {
                UnaryOperator::LogicalNot | UnaryOperator::Delete => "boolean".into(),
                UnaryOperator::Typeof => "string".into(),
                UnaryOperator::Void => "undefined".into(),
                _ => "number".into(),
            },
            Expression::BinaryExpression(b) => {
                if b.operator.is_compare() || b.operator.is_equality() || b.operator.is_in() || b.operator.is_instance_of() {
                    "boolean".into()
                } else if b.operator == BinaryOperator::Addition {
                    match (self.infer(&b.left).as_deref(), self.infer(&b.right).as_deref()) {
                        (Some("string"), _) | (_, Some("string")) => "string".into(),
                        (Some("number"), Some("number")) => "number".into(),
                        _ => return None,
                    }
                } else {
                    "number".into()
                }
            }
            _ => return None,
        })
    }

    fn annotation(&self, t: &Option<oxc_allocator::Box<TSTypeAnnotation>>) -> Option<String> {
        t.as_ref().map(|t| self.text(t.type_annotation.span(), 60))
    }

    fn params(&self, p: &FormalParameters) -> Vec<Var> {
        let mut out = Vec::new();
        for item in &p.items {
            let simple = matches!(item.pattern, BindingPattern::BindingIdentifier(_));
            let ty = self.annotation(&item.type_annotation).or_else(|| item.initializer.as_ref().and_then(|i| self.infer(i)));
            let init = item.initializer.as_ref().map(|i| self.text(i.span(), 30));
            for id in item.pattern.get_binding_identifiers() {
                out.push(Var {
                    name: id.name.to_string(),
                    kind: "param",
                    line: self.line(id.span.start),
                    ty: if simple { ty.clone() } else { None },
                    init: if simple { init.clone() } else { Some("part of an argument".into()) },
                });
            }
        }
        if let Some(r) = &p.rest {
            for id in r.rest.argument.get_binding_identifiers() {
                out.push(Var { name: id.name.to_string(), kind: "param", line: self.line(id.span.start), ty: self.annotation(&r.type_annotation), init: Some("the remaining arguments".into()) });
            }
        }
        out
    }

    /// Open a scope for a function. Named functions get their own entry;
    /// anonymous ones report into the function around them.
    #[allow(clippy::too_many_arguments)]
    fn enter(&mut self, name: Option<String>, kind: &'static str, span: Span, is_async: bool, generator: bool, params: &FormalParameters, ret: &Option<oxc_allocator::Box<TSTypeAnnotation>>, body: &[Statement]) {
        let mut d = Decls::default();
        for s in body {
            d.visit_statement(s);
        }
        let pvars = self.params(params);
        let pnames: HashSet<String> = pvars.iter().map(|p| p.name.clone()).collect();
        let Some(name) = name else {
            d.vars.extend(pnames);
            self.scopes.push(Scope { f: self.cur(), own: false, vars: d.vars, params: HashSet::new(), other: d.other, depth: self.scopes.last().map_or(0, |s| s.depth) });
            return;
        };
        let ret = self.annotation(ret);
        let mut ptext = self.text(params.span, 70);
        if !ptext.starts_with('(') {
            ptext = format!("({ptext})");
        }
        let signature = format!(
            "{}{}{name}{ptext}{}",
            if is_async { "async " } else { "" },
            if generator { "*" } else { "" },
            ret.as_ref().map(|r| format!(": {r}")).unwrap_or_default()
        );
        let level = self.scopes.iter().filter(|s| s.own).count() - 1;
        self.fns.push(Func { name, kind, signature, line: self.line(span.start), end: self.line(span.end.saturating_sub(1)), level, is_async, generator, params: pvars, ret, ..Func::default() });
        self.scopes.push(Scope { f: self.fns.len() - 1, own: true, vars: d.vars, params: pnames, other: d.other, depth: 0 });
    }

    fn export(&mut self, name: &str) {
        push_unique(&mut self.exports, name);
    }

    /// `module.exports = …`, `exports.x = …`
    fn cjs_export(&mut self, it: &AssignmentExpression) {
        let left = self.text(it.left.span(), 80);
        if let Some(name) = left.strip_prefix("module.exports.").or_else(|| left.strip_prefix("exports.")) {
            self.export(name);
        } else if left == "module.exports" {
            match &it.right {
                Expression::ObjectExpression(o) => {
                    for p in &o.properties {
                        if let ObjectPropertyKind::ObjectProperty(p) = p {
                            if let Some(n) = p.key.static_name() {
                                self.export(&n);
                            }
                        }
                    }
                }
                Expression::Identifier(id) => {
                    self.export("default");
                    self.exported.insert(id.name.to_string());
                }
                _ => self.export("default"),
            }
        }
    }

    fn call(&mut self, name: String, pos: u32) {
        let line = self.line(pos);
        let f = self.f();
        match f.calls.iter_mut().find(|c| c.name == name) {
            Some(c) => c.lines.push(line),
            None => f.calls.push(Call { name, lines: vec![line], target: None }),
        }
    }

    /// Readable name for what is being called: `f`, `obj.method`, `….method`.
    fn callee(&self, e: &Expression) -> String {
        let e = e.get_inner_expression();
        match e {
            Expression::Identifier(id) => id.name.to_string(),
            Expression::Super(_) => "super".into(),
            _ if root_of(e).is_some() && e.span().size() <= 40 => self.text(e.span(), 40).replace("?.", "."),
            Expression::StaticMemberExpression(m) => format!("….{}", m.property.name),
            _ => "(expression)".into(),
        }
    }
}

impl<'a, 's> Visit<'a> for Analyzer<'s> {
    // ---------- functions

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        let is_fn = it.init.as_ref().map_or(false, is_fn_expr);
        if is_fn {
            self.pending = it.id.get_binding_identifier().map(|b| b.name.to_string());
        }
        let iter_src = self.iter_src.take();
        if self.own() && !is_fn {
            let simple = matches!(it.id, BindingPattern::BindingIdentifier(_));
            let kind = match it.kind {
                VariableDeclarationKind::Var => "var",
                VariableDeclarationKind::Let => "let",
                VariableDeclarationKind::Const => "const",
                _ => "using",
            };
            let ty = self.annotation(&it.type_annotation).or_else(|| it.init.as_ref().and_then(|i| self.infer(i)));
            let init = match &it.init {
                Some(i) if simple => Some(self.text(i.span(), 36)),
                Some(i) => Some(format!("part of {}", self.text(i.span(), 28))),
                None => iter_src,
            };
            // `const fs = require('fs')` is an import
            if let Some(Expression::CallExpression(c)) = &it.init {
                if let (Expression::Identifier(id), Some(Argument::StringLiteral(s))) = (&c.callee, c.arguments.first()) {
                    if id.name == "require" {
                        let names = it.id.get_binding_identifiers().iter().map(|b| b.name.to_string()).collect();
                        self.imports.push(Import { from: s.value.to_string(), names, line: self.line(it.span.start) });
                    }
                }
            }
            for id in it.id.get_binding_identifiers() {
                let v = Var { name: id.name.to_string(), kind, line: self.line(id.span.start), ty: if simple { ty.clone() } else { None }, init: init.clone() };
                self.f().locals.push(v);
            }
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if is_fn_expr(&it.value) {
            self.pending = it.key.static_name().map(|n| n.to_string());
            self.kind = Some("method");
        }
        walk::walk_object_property(self, it);
    }

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        if it.value.as_ref().map_or(false, is_fn_expr) {
            let key = it.key.static_name().map(|n| n.to_string()).unwrap_or_else(|| "method".into());
            self.pending = Some(match self.classes.last() {
                Some(c) => format!("{c}.{key}"),
                None => key,
            });
            self.kind = Some("method");
        }
        walk::walk_property_definition(self, it);
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        let name = it.id.as_ref().map(|i| i.name.to_string()).or_else(|| self.pending.take()).unwrap_or_else(|| "class".into());
        self.types.push(TypeDecl { name: name.clone(), kind: "class", line: self.line(it.span.start) });
        self.classes.push(name);
        walk::walk_class(self, it);
        self.classes.pop();
    }

    fn visit_method_definition(&mut self, it: &MethodDefinition<'a>) {
        let key = it.key.static_name().map(|n| n.to_string()).unwrap_or_else(|| "method".into());
        self.pending = Some(match self.classes.last() {
            Some(c) => format!("{c}.{key}"),
            None => key,
        });
        self.kind = Some(match it.kind {
            MethodDefinitionKind::Constructor => "constructor",
            MethodDefinitionKind::Get => "getter",
            MethodDefinitionKind::Set => "setter",
            MethodDefinitionKind::Method => "method",
        });
        walk::walk_method_definition(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let name = it.id.as_ref().map(|i| i.name.to_string()).or_else(|| self.pending.take());
        self.pending = None;
        let kind = self.kind.take().unwrap_or("function");
        let Some(body) = &it.body else {
            return walk::walk_function(self, it, flags);
        };
        self.enter(name, kind, it.span, it.r#async, it.generator, &it.params, &it.return_type, &body.statements);
        walk::walk_function(self, it, flags);
        self.scopes.pop();
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let name = self.pending.take();
        let kind = self.kind.take().unwrap_or("arrow");
        let named = name.is_some();
        self.enter(name, kind, it.span, it.r#async, false, &it.params, &it.return_type, &it.body.statements);
        // `x => x + 1`: the body is the returned value
        if it.expression && named {
            if let Some(Statement::ExpressionStatement(e)) = it.body.statements.first() {
                let v = self.text(e.expression.span(), 30);
                let f = self.f();
                f.returns += 1;
                f.return_values.push(v);
            }
        }
        walk::walk_arrow_function_expression(self, it);
        self.scopes.pop();
    }

    // ---------- names, calls and effects

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        match self.resolve(&it.name) {
            Where::OuterVar => push_unique(&mut self.f().reads_outer, &it.name),
            Where::Other => {
                self.referenced.insert(it.name.to_string());
            }
            _ => {}
        }
    }

    fn visit_simple_assignment_target(&mut self, it: &SimpleAssignmentTarget<'a>) {
        match it {
            // a plain write is not a read: record it and stop
            SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => self.write(&id.name, false, false),
            _ => {
                if let Some(root) = it.as_member_expression().and_then(|m| root_of(m.object())) {
                    self.write(root, true, false);
                }
                walk::walk_simple_assignment_target(self, it);
            }
        }
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        self.cjs_export(it);
        walk::walk_assignment_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let name = self.callee(&it.callee);
        self.call(name, it.span.start);
        if let Expression::StaticMemberExpression(m) = it.callee.get_inner_expression() {
            if MUTATORS.contains(&m.property.name.as_str()) {
                if let Some(root) = root_of(&m.object) {
                    self.write(root, true, true);
                }
            }
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        let name = format!("new {}", self.callee(&it.callee));
        self.call(name, it.span.start);
        walk::walk_new_expression(self, it);
    }

    // ---------- control flow

    fn visit_if_statement(&mut self, it: &IfStatement<'a>) {
        self.f().branches += 1;
        self.visit_expression(&it.test);
        self.nest(1);
        self.visit_statement(&it.consequent);
        self.nest(-1);
        match &it.alternate {
            // `else if` continues the chain at the same depth
            Some(a @ Statement::IfStatement(_)) => self.visit_statement(a),
            Some(a) => {
                self.nest(1);
                self.visit_statement(a);
                self.nest(-1);
            }
            None => {}
        }
    }

    fn visit_conditional_expression(&mut self, it: &ConditionalExpression<'a>) {
        self.f().branches += 1;
        walk::walk_conditional_expression(self, it);
    }

    fn visit_switch_statement(&mut self, it: &SwitchStatement<'a>) {
        self.f().branches += it.cases.iter().filter(|c| c.test.is_some()).count() as u32;
        self.nest(1);
        walk::walk_switch_statement(self, it);
        self.nest(-1);
    }

    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        self.f().loops += 1;
        self.nest(1);
        walk::walk_for_statement(self, it);
        self.nest(-1);
    }

    fn visit_for_in_statement(&mut self, it: &ForInStatement<'a>) {
        self.f().loops += 1;
        self.iter_src = Some(format!("each key of {}", self.text(it.right.span(), 24)));
        self.nest(1);
        walk::walk_for_in_statement(self, it);
        self.nest(-1);
        self.iter_src = None;
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.f().loops += 1;
        if it.r#await {
            self.f().awaits += 1;
        }
        self.iter_src = Some(format!("each of {}", self.text(it.right.span(), 28)));
        self.nest(1);
        walk::walk_for_of_statement(self, it);
        self.nest(-1);
        self.iter_src = None;
    }

    fn visit_while_statement(&mut self, it: &WhileStatement<'a>) {
        self.f().loops += 1;
        self.nest(1);
        walk::walk_while_statement(self, it);
        self.nest(-1);
    }

    fn visit_do_while_statement(&mut self, it: &DoWhileStatement<'a>) {
        self.f().loops += 1;
        self.nest(1);
        walk::walk_do_while_statement(self, it);
        self.nest(-1);
    }

    fn visit_return_statement(&mut self, it: &ReturnStatement<'a>) {
        // a `return` inside a folded callback leaves the callback, not this function
        if self.own() {
            let v = it.argument.as_ref().map(|a| self.text(a.span(), 30));
            let f = self.f();
            f.returns += 1;
            if let Some(v) = v {
                if f.return_values.len() < 4 {
                    push_unique(&mut f.return_values, &v);
                }
            }
        }
        walk::walk_return_statement(self, it);
    }

    fn visit_throw_statement(&mut self, it: &ThrowStatement<'a>) {
        self.f().throws += 1;
        walk::walk_throw_statement(self, it);
    }

    fn visit_await_expression(&mut self, it: &AwaitExpression<'a>) {
        self.f().awaits += 1;
        walk::walk_await_expression(self, it);
    }

    // ---------- module shape

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        let names = it.specifiers.iter().flatten().map(|s| match s {
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(n) => format!("* as {}", n.local.name),
            other => other.name().to_string(),
        }).collect();
        self.imports.push(Import { from: it.source.value.to_string(), names, line: self.line(it.span.start) });
    }

    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        match &it.declaration {
            Some(Declaration::FunctionDeclaration(f)) => {
                if let Some(id) = &f.id {
                    self.export(&id.name);
                }
            }
            Some(Declaration::ClassDeclaration(c)) => {
                if let Some(id) = &c.id {
                    self.export(&id.name);
                }
            }
            Some(Declaration::VariableDeclaration(d)) => {
                for decl in &d.declarations {
                    for id in decl.id.get_binding_identifiers() {
                        self.export(&id.name);
                    }
                }
            }
            _ => {}
        }
        for s in &it.specifiers {
            self.export(&s.exported.name());
            self.exported.insert(s.local.name().to_string());
        }
        walk::walk_export_named_declaration(self, it);
    }

    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        self.export("default");
        match &it.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                if let Some(id) = &f.id {
                    self.exported.insert(id.name.to_string());
                }
            }
            ExportDefaultDeclarationKind::Identifier(id) => {
                self.exported.insert(id.name.to_string());
            }
            _ => {}
        }
        walk::walk_export_default_declaration(self, it);
    }

    fn visit_ts_interface_declaration(&mut self, it: &TSInterfaceDeclaration<'a>) {
        self.types.push(TypeDecl { name: it.id.name.to_string(), kind: "interface", line: self.line(it.span.start) });
    }

    fn visit_ts_type_alias_declaration(&mut self, it: &TSTypeAliasDeclaration<'a>) {
        self.types.push(TypeDecl { name: it.id.name.to_string(), kind: "type", line: self.line(it.span.start) });
    }

    fn visit_ts_enum_declaration(&mut self, it: &TSEnumDeclaration<'a>) {
        self.types.push(TypeDecl { name: it.id.name.to_string(), kind: "enum", line: self.line(it.span.start) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn func<'r>(r: &'r Report, name: &str) -> &'r Func {
        r.fns.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no function {name}"))
    }

    #[test]
    fn calls_effects_and_flow() {
        let src = "\
let total = 0;
const cache = new Map();
export function sort(arr: number[]): number[] {
  const n = arr.length;
  for (let i = 0; i < n; i++) {
    for (let j = 0; j < n - i - 1; j++) {
      if (arr[j] > arr[j + 1]) swap(arr, j, j + 1);
    }
  }
  return arr;
}
function swap(a, i, j) { const t = a[i]; a[i] = a[j]; a[j] = t; }
function count(xs) { xs.forEach((x) => { total += x; }); return cache.get(xs); }
const fib = (n) => n < 2 ? n : fib(n - 1) + fib(n - 2);
function unused() {}
sort([3, 1, 2]);
";
        let r = explain(src, "a.ts").unwrap();
        let sort = func(&r, "sort");
        assert_eq!(sort.signature, "sort(arr: number[]): number[]");
        assert_eq!((sort.loops, sort.branches, sort.depth, sort.returns), (2, 1, 3, 1));
        assert_eq!(sort.calls[0].target.as_deref(), Some("swap"));
        assert_eq!(sort.called_by, vec![TOP]);
        assert!(sort.exported && sort.mutates.is_empty());
        assert_eq!(sort.locals.iter().map(var_text).collect::<Vec<_>>(), ["n = arr.length", "i: number = 0", "j: number = 0"]);

        let swap = func(&r, "swap");
        assert_eq!(swap.mutates, ["a"]);
        assert_eq!(swap.called_by, ["sort"]);

        // the callback is folded into `count`: its write and its `return` rules apply to count
        let count = func(&r, "count");
        assert_eq!(count.writes_outer, ["total"]);
        assert_eq!(count.reads_outer, ["cache"]);
        assert_eq!(count.returns, 1);
        assert!(r.fns.iter().all(|f| !f.name.contains("arrow")));

        let fib = func(&r, "fib");
        assert!(fib.recursive && fib.tags.contains(&"self-contained".to_string()));
        assert!(func(&r, "unused").tags.contains(&"not used in this file".to_string()));
        assert!(!swap.tags.contains(&"not used in this file".to_string()));

        let top = func(&r, TOP);
        assert_eq!(top.locals.iter().map(var_text).collect::<Vec<_>>(), ["total: number = 0", "cache: Map = new Map()"]);
        assert_eq!(r.exports, ["sort"]);
    }

    #[test]
    fn classes_imports_and_types() {
        let src = "\
import fs, { join as j } from 'node:path';
const os = require('os');
interface Item { id: number }
class Stack {
  items: number[] = [];
  constructor() { this.items = []; }
  push(x: number) { this.items.push(x); return this.size(); }
  size() { return this.items.length; }
}
const s = new Stack();
s.push(1);
module.exports = { Stack };
";
        let r = explain(src, "a.ts").unwrap();
        assert_eq!(r.imports.len(), 2);
        assert_eq!(r.imports[0].names, ["fs", "j"]);
        assert_eq!(r.imports[1].from, "os");
        assert_eq!(r.types.iter().map(|t| t.kind).collect::<Vec<_>>(), ["interface", "class"]);
        let push = func(&r, "Stack.push");
        assert_eq!(push.kind, "method");
        assert_eq!(push.mutates, ["this"]);
        assert_eq!(push.calls.iter().find(|c| c.name == "this.size").unwrap().target.as_deref(), Some("Stack.size"));
        assert!(func(&r, "Stack.constructor").mutates.is_empty());
        assert_eq!(func(&r, TOP).calls[1].target.as_deref(), Some("Stack.constructor"));
        assert_eq!(r.exports, ["Stack"]);
        assert!(text(&r).contains("Stack.push(x: number)"));
    }

    #[test]
    fn rejects_other_languages_and_bad_syntax() {
        assert!(explain("def f(): pass", "a.py").is_err());
        assert!(explain("function (", "a.js").is_err());
    }
}
