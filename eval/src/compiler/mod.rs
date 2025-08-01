//! This module implements a compiler for compiling the rnix AST
//! representation to Tvix bytecode.
//!
//! A note on `unwrap()`: This module contains a lot of calls to
//! `unwrap()` or `expect(...)` on data structures returned by `rnix`.
//! The reason for this is that rnix uses the same data structures to
//! represent broken and correct ASTs, so all typed AST variants have
//! the ability to represent an incorrect node.
//!
//! However, at the time that the AST is passed to the compiler we
//! have verified that `rnix` considers the code to be correct, so all
//! variants are fulfilled. In cases where the invariant is guaranteed
//! by the code in this module, `debug_assert!` has been used to catch
//! mistakes early during development.

mod bindings;
mod import;
mod optimiser;
mod scope;

use codemap::Span;
use rnix::ast::{self, AstToken};
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use crate::chunk::Chunk;
use crate::errors::{CatchableErrorKind, Error, ErrorKind, EvalResult};
use crate::observer::CompilerObserver;
use crate::opcode::{CodeIdx, Op, Position, UpvalueIdx};
use crate::spans::ToSpan;
use crate::value::{Closure, Formals, Lambda, NixAttrs, Thunk, Value};
use crate::warnings::{EvalWarning, WarningKind};
use crate::CoercionKind;
use crate::SourceCode;

use self::scope::{LocalIdx, LocalPosition, Scope, Upvalue, UpvalueKind};

/// Represents the result of compiling a piece of Nix code. If
/// compilation was successful, the resulting bytecode can be passed
/// to the VM.
pub struct CompilationOutput {
    pub lambda: Rc<Lambda>,
    pub warnings: Vec<EvalWarning>,
    pub errors: Vec<Error>,
}

/// Represents the lambda currently being compiled.
struct LambdaCtx {
    lambda: Lambda,
    scope: Scope,
    captures_with_stack: bool,
}

impl LambdaCtx {
    fn new() -> Self {
        LambdaCtx {
            lambda: Lambda::default(),
            scope: Default::default(),
            captures_with_stack: false,
        }
    }

    fn inherit(&self) -> Self {
        LambdaCtx {
            lambda: Lambda::default(),
            scope: self.scope.inherit(),
            captures_with_stack: false,
        }
    }
}

/// When compiling functions with an argument attribute set destructuring pattern,
/// we need to do multiple passes over the declared formal arguments when setting
/// up their local bindings (similarly to `let … in` expressions and recursive
/// attribute sets. For this purpose, this struct is used to represent the two
/// kinds of formal arguments:
///
/// - `TrackedFormal::NoDefault` is always required and causes an evaluation error
///   if the corresponding attribute is missing in a function call.
/// - `TrackedFormal::WithDefault` may be missing in the passed attribute set—
///   in which case a `default_expr` will be evaluated and placed in the formal
///   argument's local variable slot.
enum TrackedFormal {
    NoDefault {
        local_idx: LocalIdx,
        pattern_entry: ast::PatEntry,
    },
    WithDefault {
        local_idx: LocalIdx,
        /// Extra phantom local used for coordinating runtime dispatching not observable to
        /// the language user. Detailed description in `compile_param_pattern()`.
        finalise_request_idx: LocalIdx,
        default_expr: ast::Expr,
        pattern_entry: ast::PatEntry,
    },
}

impl TrackedFormal {
    fn pattern_entry(&self) -> &ast::PatEntry {
        match self {
            TrackedFormal::NoDefault { pattern_entry, .. } => pattern_entry,
            TrackedFormal::WithDefault { pattern_entry, .. } => pattern_entry,
        }
    }
    fn local_idx(&self) -> LocalIdx {
        match self {
            TrackedFormal::NoDefault { local_idx, .. } => *local_idx,
            TrackedFormal::WithDefault { local_idx, .. } => *local_idx,
        }
    }
}

/// The map of globally available functions and other values that
/// should implicitly be resolvable in the global scope.
pub type GlobalsMap = FxHashMap<&'static str, Value>;

/// Set of builtins that (if they exist) should be made available in
/// the global scope, meaning that they can be accessed not just
/// through `builtins.<name>`, but directly as `<name>`. This is not
/// configurable, it is based on what Nix 2.3 exposed.
const GLOBAL_BUILTINS: &[&str] = &[
    "abort",
    "baseNameOf",
    "derivation",
    "derivationStrict",
    "dirOf",
    "fetchGit",
    "fetchMercurial",
    "fetchTarball",
    "fromTOML",
    "import",
    "isNull",
    "map",
    "placeholder",
    "removeAttrs",
    "scopedImport",
    "throw",
    "toString",
    "__curPos",
];

pub struct Compiler<'source, 'observer> {
    contexts: Vec<LambdaCtx>,
    warnings: Vec<EvalWarning>,
    errors: Vec<Error>,
    root_dir: PathBuf,

    /// Carries all known global tokens; the full set of which is
    /// created when the compiler is invoked.
    ///
    /// Each global has an associated token, which when encountered as
    /// an identifier is resolved against the scope poisoning logic,
    /// and a function that should emit code for the token.
    globals: Rc<GlobalsMap>,

    /// Reference to the struct holding all of the source code, which
    /// is used for error creation.
    source: &'source SourceCode,

    /// File reference in the source map for the current file, which
    /// is used for creating spans.
    file: &'source codemap::File,

    /// Carry an observer for the compilation process, which is called
    /// whenever a chunk is emitted.
    observer: &'observer mut dyn CompilerObserver,

    /// Carry a count of nested scopes which have requested the
    /// compiler not to emit anything. This used for compiling dead
    /// code branches to catch errors & warnings in them.
    dead_scope: usize,
}

impl Compiler<'_, '_> {
    pub(super) fn span_for<S: ToSpan>(&self, to_span: &S) -> Span {
        to_span.span_for(self.file)
    }
}

/// Compiler construction
impl<'source, 'observer> Compiler<'source, 'observer> {
    pub(crate) fn new(
        location: Option<PathBuf>,
        globals: Rc<GlobalsMap>,
        env: Option<&FxHashMap<SmolStr, Value>>,
        source: &'source SourceCode,
        file: &'source codemap::File,
        observer: &'observer mut dyn CompilerObserver,
    ) -> EvalResult<Self> {
        let mut root_dir = match location {
            Some(dir) if cfg!(target_arch = "wasm32") || dir.is_absolute() => Ok(dir),
            _ => {
                let current_dir = std::env::current_dir().map_err(|e| {
                    Error::new(
                        ErrorKind::RelativePathResolution(format!(
                            "could not determine current directory: {e}"
                        )),
                        file.span,
                        source.clone(),
                    )
                })?;
                if let Some(dir) = location {
                    Ok(current_dir.join(dir))
                } else {
                    Ok(current_dir)
                }
            }
        }?;

        // If the path passed from the caller points to a file, the
        // filename itself needs to be truncated as this must point to a
        // directory.
        if root_dir.is_file() {
            root_dir.pop();
        }

        #[cfg(not(target_arch = "wasm32"))]
        debug_assert!(root_dir.is_absolute());

        let mut compiler = Self {
            root_dir,
            source,
            file,
            observer,
            globals,
            contexts: vec![LambdaCtx::new()],
            warnings: vec![],
            errors: vec![],
            dead_scope: 0,
        };

        if let Some(env) = env {
            compiler.compile_env(env);
        }

        Ok(compiler)
    }
}

// Helper functions for emitting code and metadata to the internal
// structures of the compiler.
impl Compiler<'_, '_> {
    fn context(&self) -> &LambdaCtx {
        &self.contexts[self.contexts.len() - 1]
    }

    fn context_mut(&mut self) -> &mut LambdaCtx {
        let idx = self.contexts.len() - 1;
        &mut self.contexts[idx]
    }

    fn chunk(&mut self) -> &mut Chunk {
        &mut self.context_mut().lambda.chunk
    }

    fn scope(&self) -> &Scope {
        &self.context().scope
    }

    fn scope_mut(&mut self) -> &mut Scope {
        &mut self.context_mut().scope
    }

    /// Push a single instruction to the current bytecode chunk and
    /// track the source span from which it was compiled.
    fn push_op<T: ToSpan>(&mut self, data: Op, node: &T) -> CodeIdx {
        if self.dead_scope > 0 {
            return CodeIdx(0);
        }

        let span = self.span_for(node);
        CodeIdx(self.chunk().push_op(data, span))
    }

    fn push_u8(&mut self, data: u8) {
        if self.dead_scope > 0 {
            return;
        }

        self.chunk().code.push(data);
    }

    fn push_uvarint(&mut self, data: u64) {
        if self.dead_scope > 0 {
            return;
        }

        self.chunk().push_uvarint(data);
    }

    fn push_u16(&mut self, data: u16) {
        if self.dead_scope > 0 {
            return;
        }

        self.chunk().push_u16(data);
    }

    /// Emit a single constant to the current bytecode chunk and track
    /// the source span from which it was compiled.
    pub(super) fn emit_constant<T: ToSpan>(&mut self, value: Value, node: &T) {
        if self.dead_scope > 0 {
            return;
        }

        let idx = self.chunk().push_constant(value);
        self.push_op(Op::Constant, node);
        self.push_uvarint(idx.0 as u64);
    }
}

// Actual code-emitting AST traversal methods.
impl Compiler<'_, '_> {
    fn compile(&mut self, slot: LocalIdx, expr: ast::Expr) {
        let expr = optimiser::optimise_expr(self, slot, expr);

        match &expr {
            ast::Expr::Literal(literal) => self.compile_literal(literal),
            ast::Expr::Path(path) => self.compile_path(slot, path),
            ast::Expr::Str(s) => self.compile_str(slot, s),

            ast::Expr::UnaryOp(op) => self.thunk(slot, op, move |c, s| c.compile_unary_op(s, op)),

            ast::Expr::BinOp(binop) => {
                self.thunk(slot, binop, move |c, s| c.compile_binop(s, binop))
            }

            ast::Expr::HasAttr(has_attr) => {
                self.thunk(slot, has_attr, move |c, s| c.compile_has_attr(s, has_attr))
            }

            ast::Expr::List(list) => self.thunk(slot, list, move |c, s| c.compile_list(s, list)),

            ast::Expr::AttrSet(attrs) => {
                self.thunk(slot, attrs, move |c, s| c.compile_attr_set(s, attrs))
            }

            ast::Expr::Select(select) => {
                self.thunk(slot, select, move |c, s| c.compile_select(s, select))
            }

            ast::Expr::Assert(assert) => {
                self.thunk(slot, assert, move |c, s| c.compile_assert(s, assert))
            }
            ast::Expr::IfElse(if_else) => {
                self.thunk(slot, if_else, move |c, s| c.compile_if_else(s, if_else))
            }

            ast::Expr::LetIn(let_in) => {
                self.thunk(slot, let_in, move |c, s| c.compile_let_in(s, let_in))
            }

            ast::Expr::Ident(ident) => self.compile_ident(slot, ident),
            ast::Expr::With(with) => self.thunk(slot, with, |c, s| c.compile_with(s, with)),
            ast::Expr::Lambda(lambda) => self.thunk(slot, lambda, move |c, s| {
                c.compile_lambda_or_thunk(false, s, lambda, |c, s| c.compile_lambda(s, lambda))
            }),
            ast::Expr::Apply(apply) => {
                self.thunk(slot, apply, move |c, s| c.compile_apply(s, apply))
            }

            // Parenthesized expressions are simply unwrapped, leaving
            // their value on the stack.
            ast::Expr::Paren(paren) => self.compile(slot, paren.expr().unwrap()),

            ast::Expr::LegacyLet(legacy_let) => self.thunk(slot, legacy_let, move |c, s| {
                c.compile_legacy_let(s, legacy_let)
            }),

            ast::Expr::Root(_) => unreachable!("there cannot be more than one root"),
            ast::Expr::Error(_) => unreachable!("compile is only called on validated trees"),
        }
    }

    /// Compiles an expression, but does not emit any code for it as
    /// it is considered dead. This will still catch errors and
    /// warnings in that expression.
    ///
    /// A warning about the that code being dead is assumed to already be
    /// emitted by the caller of this.
    fn compile_dead_code(&mut self, slot: LocalIdx, node: ast::Expr) {
        self.dead_scope += 1;
        self.compile(slot, node);
        self.dead_scope -= 1;
    }

    fn compile_literal(&mut self, node: &ast::Literal) {
        let value = match node.kind() {
            ast::LiteralKind::Float(f) => Value::Float(f.value().unwrap()),
            ast::LiteralKind::Integer(i) => match i.value() {
                Ok(v) => Value::Integer(v),
                Err(err) => return self.emit_error(node, err.into()),
            },

            ast::LiteralKind::Uri(u) => {
                self.emit_warning(node, WarningKind::DeprecatedLiteralURL);
                Value::from(u.syntax().text())
            }
        };

        self.emit_constant(value, node);
    }

    fn compile_path(&mut self, slot: LocalIdx, node: &ast::Path) {
        // TODO(tazjin): placeholder implementation while waiting for
        // https://github.com/nix-community/rnix-parser/pull/96

        let raw_path = node.to_string();
        let path = if raw_path.starts_with('/') {
            Path::new(&raw_path).to_owned()
        } else if raw_path.starts_with('~') {
            // We assume that home paths start with ~/ or fail to parse
            // TODO: this should be checked using a parse-fail test.
            debug_assert!(raw_path.len() > 2 && raw_path.starts_with("~/"));

            let home_relative_path = &raw_path[2..(raw_path.len())];
            self.emit_constant(
                Value::UnresolvedPath(Box::new(home_relative_path.into())),
                node,
            );
            self.push_op(Op::ResolveHomePath, node);
            return;
        } else if raw_path.starts_with('<') {
            // TODO: decide what to do with findFile
            if raw_path.len() == 2 {
                return self.emit_constant(
                    Value::Catchable(Box::new(CatchableErrorKind::NixPathResolution(
                        "Empty <> path not allowed".into(),
                    ))),
                    node,
                );
            }
            let path = &raw_path[1..(raw_path.len() - 1)];
            // Make a thunk to resolve the path (without using `findFile`, at least for now?)
            return self.thunk(slot, node, move |c, _| {
                c.emit_constant(Value::UnresolvedPath(Box::new(path.into())), node);
                c.push_op(Op::FindFile, node);
            });
        } else {
            let mut buf = self.root_dir.clone();
            buf.push(&raw_path);
            buf
        };

        // TODO: Use https://github.com/rust-lang/rfcs/issues/2208
        // once it is available
        let value = Value::Path(Box::new(crate::value::canon_path(path)));
        self.emit_constant(value, node);
    }

    /// Helper that compiles the given string parts strictly. The caller
    /// (`compile_str`) needs to figure out if the result of compiling this
    /// needs to be thunked or not.
    fn compile_str_parts(
        &mut self,
        slot: LocalIdx,
        parent_node: &ast::Str,
        parts: Vec<ast::InterpolPart<String>>,
    ) {
        // The string parts are produced in literal order, however
        // they need to be reversed on the stack in order to
        // efficiently create the real string in case of
        // interpolation.
        for part in parts.iter().rev() {
            match part {
                // Interpolated expressions are compiled as normal and
                // dealt with by the VM before being assembled into
                // the final string. We need to coerce them here,
                // so OpInterpolate definitely has a string to consume.
                ast::InterpolPart::Interpolation(ipol) => {
                    self.compile(slot, ipol.expr().unwrap());
                    // implicitly forces as well
                    self.push_op(Op::CoerceToString, ipol);

                    let encoded: u8 = CoercionKind {
                        strong: false,
                        import_paths: true,
                    }
                    .into();

                    self.push_u8(encoded);
                }

                ast::InterpolPart::Literal(lit) => {
                    self.emit_constant(Value::from(lit.as_str()), parent_node);
                }
            }
        }

        if parts.len() != 1 {
            self.push_op(Op::Interpolate, parent_node);
            self.push_uvarint(parts.len() as u64);
        }
    }

    fn compile_str(&mut self, slot: LocalIdx, node: &ast::Str) {
        let parts = node.normalized_parts();

        // We need to thunk string expressions if they are the result of
        // interpolation. A string that only consists of a single part (`"${foo}"`)
        // can't desugar to the enclosed expression (`foo`) because we need to
        // coerce the result to a string value. This would require forcing the
        // value of the inner expression, so we need to wrap it in another thunk.
        if parts.len() != 1 || matches!(&parts[0], ast::InterpolPart::Interpolation(_)) {
            self.thunk(slot, node, move |c, s| {
                c.compile_str_parts(s, node, parts);
            });
        } else {
            self.compile_str_parts(slot, node, parts);
        }
    }

    fn compile_unary_op(&mut self, slot: LocalIdx, op: &ast::UnaryOp) {
        self.compile(slot, op.expr().unwrap());
        self.emit_force(op);

        let opcode = match op.operator().unwrap() {
            ast::UnaryOpKind::Invert => Op::Invert,
            ast::UnaryOpKind::Negate => Op::Negate,
        };

        self.push_op(opcode, op);
    }

    fn compile_binop(&mut self, slot: LocalIdx, op: &ast::BinOp) {
        use ast::BinOpKind;

        // Short-circuiting and other strange operators, which are
        // under the same node type as NODE_BIN_OP, but need to be
        // handled separately (i.e. before compiling the expressions
        // used for standard binary operators).

        match op.operator().unwrap() {
            BinOpKind::And => return self.compile_and(slot, op),
            BinOpKind::Or => return self.compile_or(slot, op),
            BinOpKind::Implication => return self.compile_implication(slot, op),
            _ => {}
        };

        // For all other operators, the two values need to be left on
        // the stack in the correct order before pushing the
        // instruction for the operation itself.
        self.compile(slot, op.lhs().unwrap());
        self.emit_force(&op.lhs().unwrap());

        self.compile(slot, op.rhs().unwrap());
        self.emit_force(&op.rhs().unwrap());

        match op.operator().unwrap() {
            BinOpKind::Add => self.push_op(Op::Add, op),
            BinOpKind::Sub => self.push_op(Op::Sub, op),
            BinOpKind::Mul => self.push_op(Op::Mul, op),
            BinOpKind::Div => self.push_op(Op::Div, op),
            BinOpKind::Update => self.push_op(Op::AttrsUpdate, op),
            BinOpKind::Equal => self.push_op(Op::Equal, op),
            BinOpKind::Less => self.push_op(Op::Less, op),
            BinOpKind::LessOrEq => self.push_op(Op::LessOrEq, op),
            BinOpKind::More => self.push_op(Op::More, op),
            BinOpKind::MoreOrEq => self.push_op(Op::MoreOrEq, op),
            BinOpKind::Concat => self.push_op(Op::Concat, op),

            BinOpKind::NotEqual => {
                self.push_op(Op::Equal, op);
                self.push_op(Op::Invert, op)
            }

            // Handled by separate branch above.
            BinOpKind::And | BinOpKind::Implication | BinOpKind::Or => {
                unreachable!()
            }
        };
    }

    fn compile_and(&mut self, slot: LocalIdx, node: &ast::BinOp) {
        debug_assert!(
            matches!(node.operator(), Some(ast::BinOpKind::And)),
            "compile_and called with wrong operator kind: {:?}",
            node.operator(),
        );

        // Leave left-hand side value on the stack.
        self.compile(slot, node.lhs().unwrap());
        self.emit_force(&node.lhs().unwrap());

        let throw_idx = self.push_op(Op::JumpIfCatchable, node);
        self.push_u16(0);
        // If this value is false, jump over the right-hand side - the
        // whole expression is false.
        let end_idx = self.push_op(Op::JumpIfFalse, node);
        self.push_u16(0);

        // Otherwise, remove the previous value and leave the
        // right-hand side on the stack. Its result is now the value
        // of the whole expression.
        self.push_op(Op::Pop, node);
        self.compile(slot, node.rhs().unwrap());
        self.emit_force(&node.rhs().unwrap());

        self.patch_jump(end_idx);
        self.push_op(Op::AssertBool, node);
        self.patch_jump(throw_idx);
    }

    fn compile_or(&mut self, slot: LocalIdx, node: &ast::BinOp) {
        debug_assert!(
            matches!(node.operator(), Some(ast::BinOpKind::Or)),
            "compile_or called with wrong operator kind: {:?}",
            node.operator(),
        );

        // Leave left-hand side value on the stack
        self.compile(slot, node.lhs().unwrap());
        self.emit_force(&node.lhs().unwrap());

        let throw_idx = self.push_op(Op::JumpIfCatchable, node);
        self.push_u16(0);
        // Opposite of above: If this value is **true**, we can
        // short-circuit the right-hand side.
        let end_idx = self.push_op(Op::JumpIfTrue, node);
        self.push_u16(0);
        self.push_op(Op::Pop, node);
        self.compile(slot, node.rhs().unwrap());
        self.emit_force(&node.rhs().unwrap());

        self.patch_jump(end_idx);
        self.push_op(Op::AssertBool, node);
        self.patch_jump(throw_idx);
    }

    fn compile_implication(&mut self, slot: LocalIdx, node: &ast::BinOp) {
        debug_assert!(
            matches!(node.operator(), Some(ast::BinOpKind::Implication)),
            "compile_implication called with wrong operator kind: {:?}",
            node.operator(),
        );

        // Leave left-hand side value on the stack and invert it.
        self.compile(slot, node.lhs().unwrap());
        self.emit_force(&node.lhs().unwrap());
        let throw_idx = self.push_op(Op::JumpIfCatchable, node);
        self.push_u16(0);
        self.push_op(Op::Invert, node);

        // Exactly as `||` (because `a -> b` = `!a || b`).
        let end_idx = self.push_op(Op::JumpIfTrue, node);
        self.push_u16(0);

        self.push_op(Op::Pop, node);
        self.compile(slot, node.rhs().unwrap());
        self.emit_force(&node.rhs().unwrap());

        self.patch_jump(end_idx);
        self.push_op(Op::AssertBool, node);
        self.patch_jump(throw_idx);
    }

    /// Compile list literals into equivalent bytecode. List
    /// construction is fairly simple, consisting of pushing code for
    /// each literal element and an instruction with the element
    /// count.
    ///
    /// The VM, after evaluating the code for each element, simply
    /// constructs the list from the given number of elements.
    fn compile_list(&mut self, slot: LocalIdx, node: &ast::List) {
        let mut count = 0;

        // Open a temporary scope to correctly account for stack items
        // that exist during the construction.
        self.scope_mut().begin_scope();

        for item in node.items() {
            // Start tracing new stack slots from the second list
            // element onwards. The first list element is located in
            // the stack slot of the list itself.
            let item_slot = match count {
                0 => slot,
                _ => {
                    let item_span = self.span_for(&item);
                    self.scope_mut().declare_phantom(item_span, false)
                }
            };

            count += 1;
            self.compile(item_slot, item);
            self.scope_mut().mark_initialised(item_slot);
        }

        self.push_op(Op::List, node);
        self.push_uvarint(count as u64);
        self.scope_mut().end_scope();
    }

    fn compile_attr(&mut self, slot: LocalIdx, node: &ast::Attr) {
        match node {
            ast::Attr::Dynamic(dynamic) => {
                self.compile(slot, dynamic.expr().unwrap());
                self.emit_force(&dynamic.expr().unwrap());
            }

            ast::Attr::Str(s) => {
                self.compile_str(slot, s);
                self.emit_force(s);
            }

            ast::Attr::Ident(ident) => self.emit_literal_ident(ident),
        }
    }

    fn compile_has_attr(&mut self, slot: LocalIdx, node: &ast::HasAttr) {
        // Put the attribute set on the stack.
        self.compile(slot, node.expr().unwrap());
        self.emit_force(node);

        // Push all path fragments with an operation for fetching the
        // next nested element, for all fragments except the last one.
        for (count, fragment) in node.attrpath().unwrap().attrs().enumerate() {
            if count > 0 {
                self.push_op(Op::AttrsTrySelect, &fragment);
                self.emit_force(&fragment);
            }

            self.compile_attr(slot, &fragment);
        }

        // After the last fragment, emit the actual instruction that
        // leaves a boolean on the stack.
        self.push_op(Op::HasAttr, node);
    }

    /// When compiling select or select_or expressions, an optimisation is
    /// possible of compiling the set emitted a constant attribute set by
    /// immediately replacing it with the actual value.
    ///
    /// We take care not to emit an error here, as that would interfere with
    /// thunking behaviour (there can be perfectly valid Nix code that accesses
    /// a statically known attribute set that is lacking a key, because that
    /// thunk is never evaluated). If anything is missing, just inform the
    /// caller that the optimisation did not take place and move on. We may want
    /// to emit warnings here in the future.
    fn optimise_select(&mut self, path: &ast::Attrpath) -> bool {
        // If compiling the set emitted a constant attribute set, the
        // associated constant can immediately be replaced with the
        // actual value.
        //
        // We take care not to emit an error here, as that would
        // interfere with thunking behaviour (there can be perfectly
        // valid Nix code that accesses a statically known attribute
        // set that is lacking a key, because that thunk is never
        // evaluated). If anything is missing, just move on. We may
        // want to emit warnings here in the future.
        if let Some((Op::Constant, op_idx)) = self.chunk().last_op() {
            let (idx, _) = self.chunk().read_uvarint(op_idx + 1);
            let constant = &mut self.chunk().constants[idx as usize];
            if let Value::Attrs(attrs) = constant {
                let mut path_iter = path.attrs();

                // Only do this optimisation if there is a *single*
                // element in the attribute path. It is extremely
                // unlikely that we'd have a static nested set.
                if let (Some(attr), None) = (path_iter.next(), path_iter.next()) {
                    // Only do this optimisation for statically known attrs.
                    if let Some(ident) = expr_static_attr_str(&attr) {
                        if let Some(selected_value) = attrs.select(ident.as_bytes()) {
                            *constant = selected_value.clone();
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    fn compile_select(&mut self, slot: LocalIdx, node: &ast::Select) {
        let set = node.expr().unwrap();
        let path = node.attrpath().unwrap();

        if node.or_token().is_some() {
            return self.compile_select_or(slot, set, path, node.default_expr().unwrap());
        }

        // Push the set onto the stack
        self.compile(slot, set.clone());
        if self.optimise_select(&path) {
            return;
        }

        // Compile each key fragment and emit access instructions.
        //
        // TODO: multi-select instruction to avoid re-pushing attrs on
        // nested selects.
        for fragment in path.attrs() {
            // Force the current set value.
            self.emit_force(&set);

            self.compile_attr(slot, &fragment);
            self.push_op(Op::AttrsSelect, &fragment);
        }
    }

    /// Compile an `or` expression into a chunk of conditional jumps.
    ///
    /// If at any point during attribute set traversal a key is
    /// missing, the `OpAttrOrNotFound` instruction will leave a
    /// special sentinel value on the stack.
    ///
    /// After each access, a conditional jump evaluates the top of the
    /// stack and short-circuits to the default value if it sees the
    /// sentinel.
    ///
    /// Code like `{ a.b = 1; }.a.c or 42` yields this bytecode and
    /// runtime stack:
    ///
    /// ```notrust
    ///            Bytecode                     Runtime stack
    ///  ┌────────────────────────────┐   ┌─────────────────────────┐
    ///  │    ...                     │   │ ...                     │
    ///  │ 5  OP_ATTRS(1)             │ → │ 5  [ { a.b = 1; }     ] │
    ///  │ 6  OP_CONSTANT("a")        │ → │ 6  [ { a.b = 1; } "a" ] │
    ///  │ 7  OP_ATTR_OR_NOT_FOUND    │ → │ 7  [ { b = 1; }       ] │
    ///  │ 8  JUMP_IF_NOT_FOUND(13)   │ → │ 8  [ { b = 1; }       ] │
    ///  │ 9  OP_CONSTANT("C")        │ → │ 9  [ { b = 1; } "c"   ] │
    ///  │ 10 OP_ATTR_OR_NOT_FOUND    │ → │ 10 [ NOT_FOUND        ] │
    ///  │ 11 JUMP_IF_NOT_FOUND(13)   │ → │ 11 [                  ] │
    ///  │ 12 JUMP(14)                │   │ ..     jumped over      │
    ///  │ 13 CONSTANT(42)            │ → │ 12 [ 42 ]               │
    ///  │ 14 ...                     │   │ ..   ....               │
    ///  └────────────────────────────┘   └─────────────────────────┘
    /// ```
    fn compile_select_or(
        &mut self,
        slot: LocalIdx,
        set: ast::Expr,
        path: ast::Attrpath,
        default: ast::Expr,
    ) {
        self.compile(slot, set);
        if self.optimise_select(&path) {
            return;
        }

        let mut jumps = vec![];

        for fragment in path.attrs() {
            self.emit_force(&fragment);
            self.compile_attr(slot, &fragment.clone());
            self.push_op(Op::AttrsTrySelect, &fragment);
            jumps.push(self.push_op(Op::JumpIfNotFound, &fragment));
            self.push_u16(0);
        }

        let final_jump = self.push_op(Op::Jump, &path);
        self.push_u16(0);

        for jump in jumps {
            self.patch_jump(jump);
        }

        // Compile the default value expression and patch the final
        // jump to point *beyond* it.
        self.compile(slot, default);
        self.patch_jump(final_jump);
    }

    /// Compile `assert` expressions using jumping instructions in the VM.
    ///
    /// ```notrust
    ///                        ┌─────────────────────┐
    ///                        │ 0  [ conditional ]  │
    ///                        │ 1   JUMP_IF_FALSE  →┼─┐
    ///                        │ 2  [  main body  ]  │ │ Jump to else body if
    ///                       ┌┼─3─←     JUMP        │ │ condition is false.
    ///  Jump over else body  ││ 4   OP_ASSERT_FAIL ←┼─┘
    ///  if condition is true.└┼─5─→     ...         │
    ///                        └─────────────────────┘
    /// ```
    fn compile_assert(&mut self, slot: LocalIdx, node: &ast::Assert) {
        // Compile the assertion condition to leave its value on the stack.
        self.compile(slot, node.condition().unwrap());
        self.emit_force(&node.condition().unwrap());

        let throw_idx = self.push_op(Op::JumpIfCatchable, node);
        self.push_u16(0);

        let then_idx = self.push_op(Op::JumpIfFalse, node);
        self.push_u16(0);

        self.push_op(Op::Pop, node);
        self.compile(slot, node.body().unwrap());

        let else_idx = self.push_op(Op::Jump, node);
        self.push_u16(0);

        self.patch_jump(then_idx);
        self.push_op(Op::Pop, node);
        self.push_op(Op::AssertFail, &node.condition().unwrap());

        self.patch_jump(else_idx);
        self.patch_jump(throw_idx);
    }

    /// Compile conditional expressions using jumping instructions in the VM.
    ///
    /// ```notrust
    ///                        ┌────────────────────┐
    ///                        │ 0  [ conditional ] │
    ///                        │ 1   JUMP_IF_FALSE →┼─┐
    ///                        │ 2  [  main body  ] │ │ Jump to else body if
    ///                       ┌┼─3─←     JUMP       │ │ condition is false.
    ///  Jump over else body  ││ 4  [  else body  ]←┼─┘
    ///  if condition is true.└┼─5─→     ...        │
    ///                        └────────────────────┘
    /// ```
    fn compile_if_else(&mut self, slot: LocalIdx, node: &ast::IfElse) {
        self.compile(slot, node.condition().unwrap());
        self.emit_force(&node.condition().unwrap());

        let throw_idx = self.push_op(Op::JumpIfCatchable, &node.condition().unwrap());
        self.push_u16(0);

        let then_idx = self.push_op(Op::JumpIfFalse, &node.condition().unwrap());
        self.push_u16(0);

        self.push_op(Op::Pop, node); // discard condition value
        self.compile(slot, node.body().unwrap());

        let else_idx = self.push_op(Op::Jump, node);
        self.push_u16(0);

        self.patch_jump(then_idx); // patch jump *to* else_body
        self.push_op(Op::Pop, node); // discard condition value
        self.compile(slot, node.else_body().unwrap());

        self.patch_jump(else_idx); // patch jump *over* else body
        self.patch_jump(throw_idx); // patch jump *over* else body
    }

    /// Compile `with` expressions by emitting instructions that
    /// pop/remove the indices of attribute sets that are implicitly
    /// in scope through `with` on the "with-stack".
    fn compile_with(&mut self, slot: LocalIdx, node: &ast::With) {
        self.scope_mut().begin_scope();
        // TODO: Detect if the namespace is just an identifier, and
        // resolve that directly (thus avoiding duplication on the
        // stack).
        self.compile(slot, node.namespace().unwrap());

        let span = self.span_for(&node.namespace().unwrap());

        // The attribute set from which `with` inherits values
        // occupies a slot on the stack, but this stack slot is not
        // directly accessible. As it must be accounted for to
        // calculate correct offsets, what we call a "phantom" local
        // is declared here.
        let local_idx = self.scope_mut().declare_phantom(span, true);
        let with_idx = self.scope().stack_index(local_idx);

        self.scope_mut().push_with();

        self.push_op(Op::PushWith, &node.namespace().unwrap());
        self.push_uvarint(with_idx.0 as u64);

        self.compile(slot, node.body().unwrap());

        self.push_op(Op::PopWith, node);
        self.scope_mut().pop_with();
        self.cleanup_scope(node);
    }

    /// Compiles pattern function arguments, such as `{ a, b }: ...`.
    ///
    /// These patterns are treated as a special case of locals binding
    /// where the attribute set itself is placed on the first stack
    /// slot of the call frame (either as a phantom, or named in case
    /// of an `@` binding), and the function call sets up the rest of
    /// the stack as if the parameters were rewritten into a `let`
    /// binding.
    ///
    /// For example:
    ///
    /// ```nix
    /// ({ a, b ? 2, c ? a * b, ... }@args: <body>)  { a = 10; }
    /// ```
    ///
    /// would be compiled similarly to a binding such as
    ///
    /// ```nix
    /// let args = { a = 10; };
    /// in let a = args.a;
    ///        b = args.a or 2;
    ///        c = args.c or a * b;
    ///    in <body>
    /// ```
    ///
    /// However, there are two properties of pattern function arguments that can
    /// not be compiled by desugaring in this way:
    ///
    /// 1. Bindings have to fail if too many arguments are provided. This is
    ///    done by emitting a special instruction that checks the set of keys
    ///    from a constant containing the expected keys.
    /// 2. Formal arguments with a default expression are (as an optimization and
    ///    because it is simpler) not wrapped in another thunk, instead compiled
    ///    and accessed separately. This means that the default expression may
    ///    never make it into the local's stack slot if the argument is provided
    ///    by the caller. We need to take this into account and skip any
    ///    operations specific to the expression like thunk finalisation in such
    ///    cases.
    fn compile_param_pattern(&mut self, pattern: &ast::Pattern) -> (Formals, CodeIdx) {
        let span = self.span_for(pattern);

        let (set_idx, pat_bind_name) = match pattern.pat_bind() {
            Some(name) => {
                let pat_bind_name = name.ident().unwrap().to_string();
                (
                    self.declare_local(&name, pat_bind_name.clone()),
                    Some(pat_bind_name),
                )
            }
            None => (self.scope_mut().declare_phantom(span, true), None),
        };

        // At call time, the attribute set is already at the top of the stack.
        self.scope_mut().mark_initialised(set_idx);
        self.emit_force(pattern);
        let throw_idx = self.push_op(Op::JumpIfCatchable, pattern);
        self.push_u16(0);

        // Evaluation fails on a type error, even if the argument(s) are unused.
        self.push_op(Op::AssertAttrs, pattern);

        let ellipsis = pattern.ellipsis_token().is_some();
        if !ellipsis {
            self.push_op(Op::ValidateClosedFormals, pattern);
        }

        // Similar to `let ... in ...`, we now do multiple passes over
        // the bindings to first declare them, then populate them, and
        // then finalise any necessary recursion into the scope.
        let mut entries: Vec<TrackedFormal> = vec![];
        let mut arguments = BTreeMap::default();

        for entry in pattern.pat_entries() {
            let ident = entry.ident().unwrap();
            let idx = self.declare_local(&ident, ident.to_string());

            arguments.insert(ident.into(), entry.default().is_some());

            if let Some(default_expr) = entry.default() {
                entries.push(TrackedFormal::WithDefault {
                    local_idx: idx,
                    // This phantom is used to track at runtime (!) whether we need to
                    // finalise the local's stack slot or not. The relevant instructions are
                    // emitted in the second pass where the mechanism is explained as well.
                    finalise_request_idx: {
                        let span = self.span_for(&default_expr);
                        self.scope_mut().declare_phantom(span, false)
                    },
                    default_expr,
                    pattern_entry: entry,
                });
            } else {
                entries.push(TrackedFormal::NoDefault {
                    local_idx: idx,
                    pattern_entry: entry,
                });
            }
        }

        // For each of the bindings, push the set on the stack and
        // attempt to select from it.
        let stack_idx = self.scope().stack_index(set_idx);
        for tracked_formal in entries.iter() {
            self.push_op(Op::GetLocal, pattern);
            self.push_uvarint(stack_idx.0 as u64);
            self.emit_literal_ident(&tracked_formal.pattern_entry().ident().unwrap());

            let idx = tracked_formal.local_idx();

            // Use the same mechanism as `compile_select_or` if a
            // default value was provided, or simply select otherwise.
            match tracked_formal {
                TrackedFormal::WithDefault {
                    default_expr,
                    pattern_entry,
                    ..
                } => {
                    // The tricky bit about compiling a formal argument with a default value
                    // is that the default may be a thunk that may depend on the value of
                    // other formal arguments, i.e. may need to be finalised. This
                    // finalisation can only happen if we are actually using the default
                    // value—otherwise OpFinalise will crash on an already finalised (or
                    // non-thunk) value.
                    //
                    // Thus we use an additional local to track whether we wound up
                    // defaulting or not. `FinaliseRequest(false)` indicates that we should
                    // not finalise, as we did not default.
                    //
                    // We are being wasteful with VM stack space in case of default
                    // expressions that don't end up needing to be finalised. Unfortunately
                    // we only know better after compiling the default expression, so
                    // avoiding unnecessary locals would mean we'd need to modify the chunk
                    // after the fact.
                    self.push_op(Op::AttrsTrySelect, &pattern_entry.ident().unwrap());
                    let jump_to_default = self.push_op(Op::JumpIfNotFound, default_expr);
                    self.push_u16(0);

                    self.emit_constant(Value::FinaliseRequest(false), default_expr);

                    let jump_over_default = self.push_op(Op::Jump, default_expr);
                    self.push_u16(0);

                    self.patch_jump(jump_to_default);

                    // Does not need to thunked since compile() already does so when necessary
                    self.compile(idx, default_expr.clone());

                    self.emit_constant(Value::FinaliseRequest(true), default_expr);

                    self.patch_jump(jump_over_default);
                }
                TrackedFormal::NoDefault { pattern_entry, .. } => {
                    self.push_op(Op::AttrsSelect, &pattern_entry.ident().unwrap());
                }
            }

            self.scope_mut().mark_initialised(idx);
            if let TrackedFormal::WithDefault {
                finalise_request_idx,
                ..
            } = tracked_formal
            {
                self.scope_mut().mark_initialised(*finalise_request_idx);
            }
        }

        for tracked_formal in entries.iter() {
            if self.scope()[tracked_formal.local_idx()].needs_finaliser {
                let stack_idx = self.scope().stack_index(tracked_formal.local_idx());
                match tracked_formal {
                    TrackedFormal::NoDefault { .. } =>
                        panic!("Tvix bug: local for pattern formal needs finaliser, but has no default expr"),
                    TrackedFormal::WithDefault { finalise_request_idx, .. } => {
                        let finalise_request_stack_idx = self.scope().stack_index(*finalise_request_idx);

                        // TODO(sterni): better spans
                        self.push_op(Op::GetLocal, pattern);
                        self.push_uvarint(finalise_request_stack_idx.0 as u64);
                        let jump_over_finalise =
                            self.push_op(Op::JumpIfNoFinaliseRequest, pattern);
                        self.push_u16(0);
                        self.push_op(Op::Finalise, pattern);
                        self.push_uvarint(stack_idx.0 as u64);
                        self.patch_jump(jump_over_finalise);
                        // Get rid of finaliser request value on the stack
                        self.push_op(Op::Pop, pattern);
                    }
                }
            }
        }

        (
            (Formals {
                arguments,
                ellipsis,
                span,
                name: pat_bind_name,
            }),
            throw_idx,
        )
    }

    fn compile_lambda(&mut self, slot: LocalIdx, node: &ast::Lambda) -> Option<CodeIdx> {
        // Compile the function itself, recording its formal arguments (if any)
        // for later use
        let formals = match node.param().unwrap() {
            ast::Param::Pattern(pat) => Some(self.compile_param_pattern(&pat)),

            ast::Param::IdentParam(param) => {
                let name = param
                    .ident()
                    .unwrap()
                    .ident_token()
                    .unwrap()
                    .text()
                    .to_string();

                let idx = self.declare_local(&param, &name);
                self.scope_mut().mark_initialised(idx);
                None
            }
        };

        self.compile(slot, node.body().unwrap());
        if let Some((formals, throw_idx)) = formals {
            self.context_mut().lambda.formals = Some(formals);
            Some(throw_idx)
        } else {
            self.context_mut().lambda.formals = None;
            None
        }
    }

    fn thunk<N, F>(&mut self, outer_slot: LocalIdx, node: &N, content: F)
    where
        N: ToSpan,
        F: FnOnce(&mut Compiler, LocalIdx),
    {
        self.compile_lambda_or_thunk(true, outer_slot, node, |comp, idx| {
            content(comp, idx);
            None
        })
    }

    /// Compile an expression into a runtime closure or thunk
    fn compile_lambda_or_thunk<N, F>(
        &mut self,
        is_suspended_thunk: bool,
        outer_slot: LocalIdx,
        node: &N,
        content: F,
    ) where
        N: ToSpan,
        F: FnOnce(&mut Compiler, LocalIdx) -> Option<CodeIdx>,
    {
        let name = self.scope()[outer_slot].name();
        self.new_context();

        // Set the (optional) name of the current slot on the lambda that is
        // being compiled.
        self.context_mut().lambda.name = name;

        let span = self.span_for(node);
        let slot = self.scope_mut().declare_phantom(span, false);
        self.scope_mut().begin_scope();

        let throw_idx = content(self, slot);
        self.cleanup_scope(node);
        if let Some(throw_idx) = throw_idx {
            self.patch_jump(throw_idx);
        }

        // Pop the lambda context back off, and emit the finished
        // lambda as a constant.
        let mut compiled = self.contexts.pop().unwrap();

        // Emit an instruction to inform the VM that the chunk has ended.
        compiled
            .lambda
            .chunk
            .push_op(Op::Return, self.span_for(node));

        let lambda = Rc::new(compiled.lambda);
        if is_suspended_thunk {
            self.observer.observe_compiled_thunk(&lambda);
        } else {
            self.observer.observe_compiled_lambda(&lambda);
        }

        // If no upvalues are captured, emit directly and move on.
        if lambda.upvalue_count == 0 && !compiled.captures_with_stack {
            self.emit_constant(
                if is_suspended_thunk {
                    Value::Thunk(Thunk::new_suspended(lambda, span))
                } else {
                    Value::Closure(Rc::new(Closure::new(lambda)))
                },
                node,
            );
            return;
        }

        // Otherwise, we need to emit the variable number of
        // operands that allow the runtime to close over the
        // upvalues and leave a blueprint in the constant index from
        // which the result can be constructed.
        let blueprint_idx = self.chunk().push_constant(Value::Blueprint(lambda));

        let code_idx = self.push_op(
            if is_suspended_thunk {
                Op::ThunkSuspended
            } else {
                Op::ThunkClosure
            },
            node,
        );
        self.push_uvarint(blueprint_idx.0 as u64);

        self.emit_upvalue_data(
            outer_slot,
            node,
            compiled.scope.upvalues,
            compiled.captures_with_stack,
        );

        if !is_suspended_thunk && !self.scope()[outer_slot].needs_finaliser {
            if !self.scope()[outer_slot].must_thunk {
                // The closure has upvalues, but is not recursive. Therefore no
                // thunk is required, which saves us the overhead of
                // Rc<RefCell<>>
                self.chunk().code[code_idx.0] = Op::Closure as u8;
            } else {
                // This case occurs when a closure has upvalue-references to
                // itself but does not need a finaliser. Since no OpFinalise
                // will be emitted later on we synthesize one here. It is needed
                // here only to set [`Closure::is_finalised`] which is used for
                // sanity checks.
                #[cfg(debug_assertions)]
                {
                    self.push_op(Op::Finalise, &self.span_for(node));
                    self.push_uvarint(self.scope().stack_index(outer_slot).0 as u64);
                }
            }
        }
    }

    fn compile_apply(&mut self, slot: LocalIdx, node: &ast::Apply) {
        // To call a function, we leave its arguments on the stack,
        // followed by the function expression itself, and then emit a
        // call instruction. This way, the stack is perfectly laid out
        // to enter the function call straight away.
        self.compile(slot, node.argument().unwrap());
        self.compile(slot, node.lambda().unwrap());
        self.emit_force(&node.lambda().unwrap());
        self.push_op(Op::Call, node);
    }

    /// Emit the data instructions that the runtime needs to correctly
    /// assemble the upvalues struct.
    fn emit_upvalue_data<T: ToSpan>(
        &mut self,
        slot: LocalIdx,
        _: &T, // TODO
        upvalues: Vec<Upvalue>,
        capture_with: bool,
    ) {
        // Push the count of arguments to be expected, with one bit set to
        // indicate whether the with stack needs to be captured.
        let mut count = (upvalues.len() as u64) << 1;
        if capture_with {
            count |= 1;
        }
        self.push_uvarint(count);

        for upvalue in upvalues {
            match upvalue.kind {
                UpvalueKind::Local(idx) => {
                    let target = &self.scope()[idx];
                    let stack_idx = self.scope().stack_index(idx);

                    // If the target is not yet initialised, we need to defer
                    // the local access
                    if !target.initialised {
                        self.push_uvarint(Position::deferred_local(stack_idx).0);
                        self.scope_mut().mark_needs_finaliser(slot);
                    } else {
                        // a self-reference
                        if slot == idx {
                            self.scope_mut().mark_must_thunk(slot);
                        }
                        self.push_uvarint(Position::stack_index(stack_idx).0);
                    }
                }

                UpvalueKind::Upvalue(idx) => {
                    self.push_uvarint(Position::upvalue_index(idx).0);
                }
            };
        }
    }

    /// Emit the literal string value of an identifier. Required for
    /// several operations related to attribute sets, where
    /// identifiers are used as string keys.
    fn emit_literal_ident(&mut self, ident: &ast::Ident) {
        self.emit_constant(Value::String(ident.clone().into()), ident);
    }

    /// Patch the jump instruction at the given index, setting its
    /// jump offset from the placeholder to the current code position.
    ///
    /// This is required because the actual target offset of jumps is
    /// not known at the time when the jump operation itself is
    /// emitted.
    fn patch_jump(&mut self, idx: CodeIdx) {
        self.chunk().patch_jump(idx.0);
    }

    /// Decrease scope depth of the current function and emit
    /// instructions to clean up the stack at runtime.
    fn cleanup_scope<N: ToSpan>(&mut self, node: &N) {
        // When ending a scope, all corresponding locals need to be
        // removed, but the value of the body needs to remain on the
        // stack. This is implemented by a separate instruction.
        let (popcount, unused_spans) = self.scope_mut().end_scope();

        for span in &unused_spans {
            self.emit_warning(span, WarningKind::UnusedBinding);
        }

        if popcount > 0 {
            self.push_op(Op::CloseScope, node);
            self.push_uvarint(popcount as u64);
        }
    }

    /// Open a new lambda context within which to compile a function,
    /// closure or thunk.
    fn new_context(&mut self) {
        self.contexts.push(self.context().inherit());
    }

    /// Declare a local variable known in the scope that is being
    /// compiled by pushing it to the locals. This is used to
    /// determine the stack offset of variables.
    fn declare_local<S: Into<String>, N: ToSpan>(&mut self, node: &N, name: S) -> LocalIdx {
        let name = name.into();
        let depth = self.scope().scope_depth();

        // Do this little dance to turn name:&'a str into the same
        // string with &'static lifetime, as required by WarningKind
        if let Some((global_ident, _)) = self.globals.get_key_value(name.as_str()) {
            self.emit_warning(node, WarningKind::ShadowedGlobal(global_ident));
        }

        let span = self.span_for(node);
        let (idx, shadowed) = self.scope_mut().declare_local(name, span);

        if let Some(shadow_idx) = shadowed {
            let other = &self.scope()[shadow_idx];
            if other.depth == depth {
                self.emit_error(node, ErrorKind::VariableAlreadyDefined(other.span));
            }
        }

        idx
    }

    /// Determine whether the current lambda context has any ancestors
    /// that use dynamic scope resolution, and mark contexts as
    /// needing to capture their enclosing `with`-stack in their
    /// upvalues.
    fn has_dynamic_ancestor(&mut self) -> bool {
        let mut ancestor_has_with = false;

        for ctx in self.contexts.iter_mut() {
            if ancestor_has_with {
                // If the ancestor has an active with stack, mark this
                // lambda context as needing to capture it.
                ctx.captures_with_stack = true;
            } else {
                // otherwise, check this context and move on
                ancestor_has_with = ctx.scope.has_with();
            }
        }

        ancestor_has_with
    }

    fn emit_force<N: ToSpan>(&mut self, node: &N) {
        self.push_op(Op::Force, node);
    }

    fn emit_warning<N: ToSpan>(&mut self, node: &N, kind: WarningKind) {
        let span = self.span_for(node);
        self.warnings.push(EvalWarning { kind, span })
    }

    fn emit_error<N: ToSpan>(&mut self, node: &N, kind: ErrorKind) {
        let span = self.span_for(node);
        self.errors
            .push(Error::new(kind, span, self.source.clone()))
    }
}

/// Convert a non-dynamic string expression to a string if possible.
fn expr_static_str(node: &ast::Str) -> Option<SmolStr> {
    let mut parts = node.normalized_parts();

    if parts.len() != 1 {
        return None;
    }

    if let Some(ast::InterpolPart::Literal(lit)) = parts.pop() {
        return Some(SmolStr::new(lit));
    }

    None
}

/// Convert the provided `ast::Attr` into a statically known string if
/// possible.
fn expr_static_attr_str(node: &ast::Attr) -> Option<SmolStr> {
    match node {
        ast::Attr::Ident(ident) => Some(ident.ident_token().unwrap().text().into()),
        ast::Attr::Str(s) => expr_static_str(s),

        // The dynamic node type is just a wrapper. C++ Nix does not care
        // about the dynamic wrapper when determining whether the node
        // itself is dynamic, it depends solely on the expression inside
        // (i.e. `let ${"a"} = 1; in a` is valid).
        ast::Attr::Dynamic(ref dynamic) => match dynamic.expr().unwrap() {
            ast::Expr::Str(s) => expr_static_str(&s),
            _ => None,
        },
    }
}

/// Create a delayed source-only builtin compilation, for a builtin
/// which is written in Nix code.
///
/// **Important:** tvix *panics* if a builtin with invalid source code
/// is supplied. This is because there is no user-friendly way to
/// thread the errors out of this function right now.
fn compile_src_builtin(
    name: &'static str,
    code: &str,
    source: SourceCode,
    weak: &Weak<GlobalsMap>,
) -> Value {
    use std::fmt::Write;

    let parsed = rnix::ast::Root::parse(code);

    if !parsed.errors().is_empty() {
        let mut out = format!("BUG: code for source-builtin '{name}' had parser errors");
        for error in parsed.errors() {
            writeln!(out, "{error}").unwrap();
        }

        panic!("{}", out);
    }

    let file = source.add_file(format!("<src-builtins/{name}.nix>"), code.to_string());
    let weak = weak.clone();

    Value::Thunk(Thunk::new_suspended_native(Box::new(move || {
        let result = compile(
            &parsed.tree().expr().unwrap(),
            None,
            weak.upgrade().unwrap(),
            None,
            &source,
            &file,
            &mut crate::observer::NoOpObserver {},
        )
        .map_err(|e| ErrorKind::NativeError {
            gen_type: "derivation",
            err: Box::new(e),
        })?;

        if !result.errors.is_empty() {
            return Err(ErrorKind::ImportCompilerError {
                path: format!("src-builtins/{name}.nix").into(),
                errors: result.errors,
            });
        }

        Ok(Value::Thunk(Thunk::new_suspended(result.lambda, file.span)))
    })))
}

/// Prepare the full set of globals available in evaluated code. These
/// are constructed from the set of builtins supplied by the caller,
/// which are made available globally under the `builtins` identifier.
///
/// A subset of builtins (specified by [`GLOBAL_BUILTINS`]) is
/// available globally *iff* they are set.
///
/// Optionally adds the `import` feature if desired by the caller.
pub fn prepare_globals(
    builtins: Vec<(&'static str, Value)>,
    src_builtins: Vec<(&'static str, &'static str)>,
    source: SourceCode,
    enable_import: bool,
) -> Rc<GlobalsMap> {
    Rc::new_cyclic(Box::new(move |weak: &Weak<GlobalsMap>| {
        // First step is to construct the builtins themselves as
        // `NixAttrs`.
        let mut builtins: GlobalsMap = FxHashMap::from_iter(builtins);

        // At this point, optionally insert `import` if enabled. To
        // "tie the knot" of `import` needing the full set of globals
        // to instantiate its compiler, the `Weak` reference is passed
        // here.
        if enable_import {
            let import = Value::Builtin(import::builtins_import(weak, source.clone()));
            builtins.insert("import", import);
        }

        // Next, the actual map of globals which the compiler will use
        // to resolve identifiers is constructed.
        let mut globals: GlobalsMap = FxHashMap::default();

        // builtins contain themselves (`builtins.builtins`), which we
        // can resolve by manually constructing a suspended thunk that
        // dereferences the same weak pointer as above.
        let weak_globals = weak.clone();
        builtins.insert(
            "builtins",
            Value::Thunk(Thunk::new_suspended_native(Box::new(move || {
                Ok(weak_globals
                    .upgrade()
                    .unwrap()
                    .get("builtins")
                    .cloned()
                    .unwrap())
            }))),
        );

        // Insert top-level static value builtins.
        globals.insert("true", Value::Bool(true));
        globals.insert("false", Value::Bool(false));
        globals.insert("null", Value::Null);

        // If "source builtins" were supplied, compile them and insert
        // them.
        builtins.extend(src_builtins.into_iter().map(move |(name, code)| {
            let compiled = compile_src_builtin(name, code, source.clone(), weak);
            (name, compiled)
        }));

        // Construct the actual `builtins` attribute set and insert it
        // in the global scope.
        globals.insert(
            "builtins",
            Value::attrs(NixAttrs::from_iter(builtins.clone())),
        );

        // Finally, the builtins that should be globally available are
        // "elevated" to the outer scope.
        for global in GLOBAL_BUILTINS {
            if let Some(builtin) = builtins.get(global).cloned() {
                globals.insert(global, builtin);
            }
        }

        globals
    }))
}

pub fn compile(
    expr: &ast::Expr,
    location: Option<PathBuf>,
    globals: Rc<GlobalsMap>,
    env: Option<&FxHashMap<SmolStr, Value>>,
    source: &SourceCode,
    file: &codemap::File,
    observer: &mut dyn CompilerObserver,
) -> EvalResult<CompilationOutput> {
    let mut c = Compiler::new(location, globals.clone(), env, source, file, observer)?;

    let root_span = c.span_for(expr);
    let root_slot = c.scope_mut().declare_phantom(root_span, false);
    c.compile(root_slot, expr.clone());

    // The final operation of any top-level Nix program must always be
    // `OpForce`. A thunk should not be returned to the user in an
    // unevaluated state (though in practice, a value *containing* a
    // thunk might be returned).
    c.emit_force(expr);
    if let Some(env) = env {
        if !env.is_empty() {
            c.push_op(Op::CloseScope, &root_span);
            c.push_uvarint(env.len() as u64);
        }
    }
    c.push_op(Op::Return, &root_span);

    let lambda = Rc::new(c.contexts.pop().unwrap().lambda);
    c.observer.observe_compiled_toplevel(&lambda);

    Ok(CompilationOutput {
        lambda,
        warnings: c.warnings,
        errors: c.errors,
    })
}
