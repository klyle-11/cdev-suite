//! JS/TS auto-watch: rewrite source so every function reports its locals
//! after each statement — the JS equivalent of Python's line tracer.
//!
//! Only insertions are made (nothing is removed or reformatted), all on the
//! same lines as the original code, so line numbers and stack traces stay put:
//!
//! ```text
//! function sort(arr) {                  function sort(arr) {const __cdev_f=__cdev_enter("sort","a.js");
//!   let i = 0;                    →       let i = 0;;__cdev_s(__cdev_f,2,{arr:()=>arr,i:()=>i});
//!   if (x) i++;                           if (x) {i++;;__cdev_s(…)};__cdev_s(…)
//! ```
//!
//! Each variable is read through its own getter so a `let` still in its
//! temporal dead zone (or a name from a sibling block) is simply skipped by
//! the runtime instead of throwing. The runtime (`__cdev_enter` / `__cdev_s`)
//! lives in the Node and browser SDKs; a prologue makes both no-ops when the
//! SDK isn't loaded.

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::scope::ScopeFlags;

const PROLOGUE: &str = "var __cdev_s=globalThis.__cdev_s||function(){},__cdev_enter=globalThis.__cdev_enter||function(){return 0};";

/// Instrument `src`; `file` picks the dialect (.ts/.tsx/.jsx/.mjs…) and labels locations.
pub fn instrument(src: &str, file: &str) -> Result<String, String> {
    // idempotent: loaders can be chained (e.g. Node runs preloads in its loader thread too)
    if src.contains(PROLOGUE) {
        return Ok(src.to_string());
    }
    let alloc = Allocator::default();
    let st = SourceType::from_path(file).unwrap_or_else(|_| SourceType::mjs());
    let ret = Parser::new(&alloc, src, st).parse();
    if ret.panicked || !ret.errors.is_empty() {
        return Err(ret.errors.first().map(|e| e.to_string()).unwrap_or_else(|| "parse failed".into()));
    }
    let program = &ret.program;
    let mut ins = Inserter { file: file.to_string(), lines: line_starts(src), edits: Vec::new(), pending: None, classes: Vec::new() };

    let start = program.directives.last().map(|d| d.span.end)
        .or_else(|| program.hashbang.as_ref().map(|h| h.span.end))
        .unwrap_or(0);
    ins.edit(start, PROLOGUE.to_string());
    ins.body(&program.body, Vec::new(), "main", start);
    ins.visit_program(program);

    // apply insertions; same position keeps insertion order
    let mut edits = ins.edits;
    edits.sort_by_key(|(pos, seq, _)| (*pos, *seq));
    let mut out = String::with_capacity(src.len() + edits.iter().map(|e| e.2.len()).sum::<usize>());
    let mut last = 0usize;
    for (pos, _, text) in edits {
        let pos = pos as usize;
        out.push_str(&src[last..pos]);
        out.push_str(&text);
        last = pos;
    }
    out.push_str(&src[last..]);
    Ok(out)
}

fn line_starts(src: &str) -> Vec<u32> {
    std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i as u32 + 1)).collect()
}

struct Inserter {
    file: String,
    lines: Vec<u32>,
    edits: Vec<(u32, usize, String)>,
    /// name for the next function literal (from `const f = …`, methods, object keys)
    pending: Option<String>,
    classes: Vec<String>,
}

impl Inserter {
    fn edit(&mut self, pos: u32, text: String) {
        let seq = self.edits.len();
        self.edits.push((pos, seq, text));
    }

    fn line(&self, pos: u32) -> usize {
        self.lines.partition_point(|&s| s <= pos)
    }

    /// Instrument one function (or the program): declare the frame at `start`,
    /// then snapshot after every statement.
    fn body(&mut self, stmts: &[Statement], mut names: Vec<String>, fname: &str, start: u32) {
        collect_decls(stmts, &mut names);
        names.retain(|n| !n.starts_with("__cdev"));
        names.dedup();
        let mut seen = std::collections::HashSet::new();
        names.retain(|n| seen.insert(n.clone()));
        if names.is_empty() {
            return;
        }
        let getters = format!("{{{}}}", names.iter().map(|n| format!("{n}:()=>{n}")).collect::<Vec<_>>().join(","));
        let enter = format!(
            "const __cdev_f=__cdev_enter({},{});",
            serde_json::to_string(fname).unwrap(),
            serde_json::to_string(&self.file).unwrap()
        );
        self.edit(start, enter);
        self.list(stmts, &getters);
    }

    fn snap(&self, line: usize, getters: &str) -> String {
        format!(";__cdev_s(__cdev_f,{line},{getters});")
    }

    fn list(&mut self, stmts: &[Statement], g: &str) {
        for s in stmts {
            self.stmt(s, g);
            if snapshot_after(s) {
                let text = self.snap(self.line(s.span().start), g);
                self.edit(s.span().end, text);
            }
        }
    }

    /// Recurse into statement containers (never into expressions: nested
    /// functions are found by the visitor and instrumented on their own).
    fn stmt(&mut self, s: &Statement, g: &str) {
        match s {
            Statement::BlockStatement(b) => self.list(&b.body, g),
            Statement::IfStatement(i) => {
                self.sub(&i.consequent, g, false);
                if let Some(a) = &i.alternate {
                    self.sub(a, g, false);
                }
            }
            Statement::ForStatement(f) => self.sub(&f.body, g, true),
            Statement::ForInStatement(f) => self.sub(&f.body, g, true),
            Statement::ForOfStatement(f) => self.sub(&f.body, g, true),
            Statement::WhileStatement(w) => self.sub(&w.body, g, true),
            Statement::DoWhileStatement(d) => self.sub(&d.body, g, true),
            Statement::LabeledStatement(l) => self.stmt(&l.body, g),
            Statement::TryStatement(t) => {
                self.list(&t.block.body, g);
                if let Some(h) = &t.handler {
                    self.list(&h.body.body, g);
                }
                if let Some(f) = &t.finalizer {
                    self.list(&f.body, g);
                }
            }
            Statement::SwitchStatement(sw) => {
                for c in &sw.cases {
                    self.list(&c.consequent, g);
                }
            }
            _ => {}
        }
    }

    /// Body of an if/loop. Blocks recurse; single statements get braces so the
    /// snapshot stays inside the branch. Loop bodies also snapshot on entry
    /// (the loop variable's new value).
    fn sub(&mut self, s: &Statement, g: &str, is_loop: bool) {
        let entry = if is_loop { self.snap(self.line(s.span().start), g) } else { String::new() };
        match s {
            Statement::BlockStatement(b) => {
                if is_loop {
                    self.edit(b.span.start + 1, entry);
                }
                self.list(&b.body, g);
            }
            Statement::IfStatement(_) if !is_loop => self.stmt(s, g), // `else if`
            _ => {
                self.edit(s.span().start, format!("{{{entry}"));
                self.stmt(s, g);
                let after = if snapshot_after(s) { self.snap(self.line(s.span().start), g) } else { String::new() };
                self.edit(s.span().end, format!("{after}}}"));
            }
        }
    }

    fn function_start(body: &FunctionBody) -> u32 {
        body.directives.last().map(|d| d.span.end).unwrap_or(body.span.start + 1)
    }
}

fn snapshot_after(s: &Statement) -> bool {
    match s {
        Statement::ExpressionStatement(_)
        | Statement::VariableDeclaration(_)
        | Statement::IfStatement(_)
        | Statement::ForStatement(_)
        | Statement::ForInStatement(_)
        | Statement::ForOfStatement(_)
        | Statement::WhileStatement(_)
        | Statement::DoWhileStatement(_)
        | Statement::TryStatement(_)
        | Statement::SwitchStatement(_)
        | Statement::BlockStatement(_)
        | Statement::LabeledStatement(_) => true,
        Statement::ExportNamedDeclaration(e) => matches!(e.declaration, Some(Declaration::VariableDeclaration(_))),
        _ => false,
    }
}

fn pattern_names(p: &BindingPattern, out: &mut Vec<String>) {
    for id in p.get_binding_identifiers() {
        out.push(id.name.to_string());
    }
}

fn var_decl_names(d: &VariableDeclaration, out: &mut Vec<String>) {
    for decl in &d.declarations {
        pattern_names(&decl.id, out);
    }
}

/// Every variable declared in these statements (not inside nested functions).
fn collect_decls(stmts: &[Statement], out: &mut Vec<String>) {
    for s in stmts {
        collect_stmt(s, out);
    }
}

fn collect_stmt(s: &Statement, out: &mut Vec<String>) {
    match s {
        Statement::VariableDeclaration(d) => var_decl_names(d, out),
        Statement::ExportNamedDeclaration(e) => {
            if let Some(Declaration::VariableDeclaration(d)) = &e.declaration {
                var_decl_names(d, out);
            }
        }
        Statement::BlockStatement(b) => collect_decls(&b.body, out),
        Statement::IfStatement(i) => {
            collect_stmt(&i.consequent, out);
            if let Some(a) = &i.alternate {
                collect_stmt(a, out);
            }
        }
        Statement::ForStatement(f) => {
            if let Some(ForStatementInit::VariableDeclaration(d)) = &f.init {
                var_decl_names(d, out);
            }
            collect_stmt(&f.body, out);
        }
        Statement::ForInStatement(f) => {
            if let ForStatementLeft::VariableDeclaration(d) = &f.left {
                var_decl_names(d, out);
            }
            collect_stmt(&f.body, out);
        }
        Statement::ForOfStatement(f) => {
            if let ForStatementLeft::VariableDeclaration(d) = &f.left {
                var_decl_names(d, out);
            }
            collect_stmt(&f.body, out);
        }
        Statement::WhileStatement(w) => collect_stmt(&w.body, out),
        Statement::DoWhileStatement(d) => collect_stmt(&d.body, out),
        Statement::LabeledStatement(l) => collect_stmt(&l.body, out),
        Statement::TryStatement(t) => {
            collect_decls(&t.block.body, out);
            if let Some(h) = &t.handler {
                if let Some(p) = &h.param {
                    pattern_names(&p.pattern, out);
                }
                collect_decls(&h.body.body, out);
            }
            if let Some(f) = &t.finalizer {
                collect_decls(&f.body, out);
            }
        }
        Statement::SwitchStatement(sw) => {
            for c in &sw.cases {
                collect_decls(&c.consequent, out);
            }
        }
        _ => {}
    }
}

fn param_names(p: &FormalParameters) -> Vec<String> {
    let mut out = Vec::new();
    for item in &p.items {
        pattern_names(&item.pattern, &mut out);
    }
    if let Some(r) = &p.rest {
        pattern_names(&r.rest.argument, &mut out);
    }
    out
}

fn is_fn_expr(e: &Expression) -> bool {
    matches!(e, Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_))
}

impl<'a> Visit<'a> for Inserter {
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if it.init.as_ref().map_or(false, is_fn_expr) {
            self.pending = it.id.get_binding_identifier().map(|b| b.name.to_string());
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if is_fn_expr(&it.value) {
            self.pending = it.key.static_name().map(|n| n.to_string());
        }
        walk::walk_object_property(self, it);
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        self.classes.push(it.id.as_ref().map(|i| i.name.to_string()).unwrap_or_else(|| "class".into()));
        walk::walk_class(self, it);
        self.classes.pop();
    }

    fn visit_method_definition(&mut self, it: &MethodDefinition<'a>) {
        let key = it.key.static_name().map(|n| n.to_string()).unwrap_or_else(|| "method".into());
        self.pending = Some(match self.classes.last() {
            Some(c) => format!("{c}.{key}"),
            None => key,
        });
        walk::walk_method_definition(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let name = it.id.as_ref().map(|i| i.name.to_string()).or_else(|| self.pending.take())
            .unwrap_or_else(|| format!("anonymous:{}", self.line(it.span.start)));
        self.pending = None;
        if let Some(body) = &it.body {
            let start = Self::function_start(body);
            self.body(&body.statements, param_names(&it.params), &name, start);
        }
        walk::walk_function(self, it, flags);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let name = self.pending.take().unwrap_or_else(|| format!("arrow:{}", self.line(it.span.start)));
        // expression-bodied arrows (`x => x + 1`) have no statements to watch
        if !it.expression {
            let start = Self::function_start(&it.body);
            self.body(&it.body.statements, param_names(&it.params), &name, start);
        }
        walk::walk_arrow_function_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_on_same_lines() {
        let src = "function sort(arr) {\n  let i = 0;\n  if (arr.length) i++;\n  for (const x of arr) { i += x; }\n  return i;\n}\n";
        let out = instrument(src, "a.js").unwrap();
        assert_eq!(out.lines().count(), src.lines().count(), "line count must not change:\n{out}");
        assert!(out.contains(r#"__cdev_enter("sort","a.js")"#));
        assert!(out.contains("{arr:()=>arr,i:()=>i,x:()=>x}"));
        assert!(out.contains("if (arr.length) {i++;;__cdev_s(__cdev_f,3,"));
        // result must still parse
        instrument(&out, "a.js").unwrap();
    }

    #[test]
    fn typescript_and_names() {
        let src = "const add = (a: number, b: number): number => {\n  const s = a + b;\n  return s;\n};\nclass K { m(x: string) { let y = x; return y; } }\n";
        let out = instrument(src, "a.ts").unwrap();
        assert!(out.contains(r#"__cdev_enter("add","a.ts")"#));
        assert!(out.contains(r#"__cdev_enter("K.m","a.ts")"#));
        instrument(&out, "a.ts").unwrap();
    }

    #[test]
    fn idempotent() {
        let once = instrument("function f(a){ let b = a; }\n", "a.js").unwrap();
        assert_eq!(instrument(&once, "a.js").unwrap(), once);
    }

    #[test]
    fn directives_and_parse_errors() {
        let out = instrument("'use strict';\nlet a = 1;\n", "a.js").unwrap();
        assert!(out.starts_with("'use strict';var __cdev_s"));
        assert!(instrument("function (", "a.js").is_err());
    }
}
