//! Rewrites method and constructor bodies:
//!
//! | user writes                | becomes                                                   |
//! | -------------------------- | --------------------------------------------------------- |
//! | `this.f` (read)            | `get(field(this.__roost_field_Owner_f(), "Owner", "f"))`: a clone, short borrow |
//! | `this.f.m(..)`             | `(&get(..)).m(..)`: `&mut self` methods don't compile      |
//! | `this.f = e`               | `set(field(..), e)`                                       |
//! | `this.f += e` (any op=)    | `{ let mut v = get(..); v += e; set(.., v) }`              |
//! | `super.m(a)`               | `<dyn OwnerClass>::__roost_method_m(this, a)`              |
//! | `this.p(a)`, `p` private   | `<dyn ClassClass>::__roost_private_p(this, a)`             |
//!
//! `Owner` is the class that declares `f` / implements `m`, resolved here at expansion time.
//! No `GcCell` guard ever outlives the helper call, so a virtual call can never run while a
//! field is borrowed: the spike's "base class holds `borrow_mut`, override reads the same
//! field" panic cannot happen. Inside macro invocations (`format!(..)`) the arguments are
//! rewritten too when they parse as expressions; otherwise only field reads are.

use proc_macro2::{Delimiter, Group, Span, TokenStream, TokenTree};
use quote::{ToTokens, quote_spanned};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit_mut::{self, VisitMut};
use syn::{BinOp, Expr, ExprMethodCall, Ident, Member, Token, parse_quote_spanned};

use crate::index::{self, Index};

pub struct Rewriter<'a> {
    pub index: &'a Index,
    pub class: usize,
    pub errors: &'a mut Vec<syn::Error>,
}

/// `this.f` -> `f`.
fn this_field(e: &Expr) -> Option<&Ident> {
    let Expr::Field(f) = e else { return None };
    let Member::Named(name) = &f.member else {
        return None;
    };
    is_path(&f.base, "this").then_some(name)
}

/// The root of a place expression (`this.f.g[0]` -> `this.f`), if it is `this.f`.
fn place_root_field(e: &Expr) -> Option<&Ident> {
    match e {
        Expr::Field(f) => this_field(e).or_else(|| place_root_field(&f.base)),
        Expr::Index(i) => place_root_field(&i.expr),
        Expr::Paren(p) => place_root_field(&p.expr),
        Expr::Unary(u) => place_root_field(&u.expr),
        _ => None,
    }
}

fn is_path(e: &Expr, name: &str) -> bool {
    matches!(e, Expr::Path(p) if p.qself.is_none() && p.path.is_ident(name))
}

/// `super` as a single-segment path. (`Path::is_ident` is false for keywords.)
fn is_super(e: &Expr) -> bool {
    match e {
        Expr::Path(p) => {
            p.qself.is_none()
                && p.path.leading_colon.is_none()
                && p.path.segments.len() == 1
                && p.path.segments[0].ident == "super"
                && p.path.segments[0].arguments.is_none()
        }
        _ => false,
    }
}

fn compound_to_plain(op: &BinOp) -> bool {
    matches!(
        op,
        BinOp::AddAssign(_)
            | BinOp::SubAssign(_)
            | BinOp::MulAssign(_)
            | BinOp::DivAssign(_)
            | BinOp::RemAssign(_)
            | BinOp::BitXorAssign(_)
            | BinOp::BitAndAssign(_)
            | BinOp::BitOrAssign(_)
            | BinOp::ShlAssign(_)
            | BinOp::ShrAssign(_)
    )
}

impl Rewriter<'_> {
    fn error(&mut self, span: Span, msg: impl std::fmt::Display) {
        self.errors.push(syn::Error::new(span, msg));
    }

    /// `field(this.__roost_field_Owner_f(), "Owner", "f")` (the names are for runtime error
    /// messages), or `None` (error recorded) if `f` doesn't resolve.
    fn cell(&mut self, name: &Ident) -> Option<TokenStream> {
        match self.index.resolve_field(self.class, name) {
            Ok(f) => {
                let owner = &self.index.classes[f.owner];
                let accessor = index::accessor_name(owner, name);
                let accessor = Ident::new(&accessor.to_string(), name.span());
                let (class, field) = (owner.name.to_string(), name.to_string());
                Some(quote_spanned!(name.span()=>
                    ::roost::__private::field(this.#accessor(), #class, #field)))
            }
            Err(msg) => {
                self.error(name.span(), msg);
                None
            }
        }
    }

    fn read(&mut self, name: &Ident, span: Span) -> Expr {
        match self.cell(name) {
            Some(cell) => parse_quote_spanned!(span=> ::roost::__private::get(#cell)),
            None => parse_quote_spanned!(span=> ::core::unreachable!()),
        }
    }

    fn visit_args(&mut self, call: &mut ExprMethodCall) {
        for arg in call.args.iter_mut() {
            self.visit_expr_mut(arg);
        }
    }

    fn rewrite_method_call(&mut self, call: &mut ExprMethodCall) -> Option<Expr> {
        let span = call.span();
        let method = call.method.clone();
        if is_super(&call.receiver) {
            self.visit_args(call);
            let Some(parent) = self.index.parent(self.class) else {
                self.error(
                    call.receiver.span(),
                    "`super` method call in a class without a superclass",
                );
                return None;
            };
            let Some(owner) = self.index.resolve_virtual(parent, &method) else {
                self.error(
                    method.span(),
                    format!("no method `{method}` in any superclass"),
                );
                return None;
            };
            let tr = index::trait_name(&self.index.classes[owner]);
            let f = index::method_impl_name(&method);
            let f = Ident::new(&f.to_string(), method.span());
            let args = &call.args;
            return Some(parse_quote_spanned!(span=> <dyn #tr>::#f(this, #args)));
        }
        if is_path(&call.receiver, "this")
            && self.index.private_method(self.class, &method).is_some()
        {
            self.visit_args(call);
            let tr = index::trait_name(&self.index.classes[self.class]);
            let f = index::private_impl_name(&method);
            let f = Ident::new(&f.to_string(), method.span());
            let args = &call.args;
            return Some(parse_quote_spanned!(span=> <dyn #tr>::#f(this, #args)));
        }
        if let Some(name) = this_field(&call.receiver).cloned() {
            // A method called on a field operates on a *copy*. Make the copy immutable, so a
            // `&mut self` method (`this.items.push(x)`) is a compile error instead of a
            // silently lost update.
            self.visit_args(call);
            let read = self.read(&name, call.receiver.span());
            *call.receiver = parse_quote_spanned!(call.receiver.span()=> (&#read));
            return None;
        }
        visit_mut::visit_expr_method_call_mut(self, call);
        None
    }

    /// Token-level fallback for macro arguments that don't parse as expressions: rewrites
    /// field reads `this.f` (not followed by `(`) and nothing else.
    fn rewrite_tokens(&mut self, tokens: TokenStream) -> TokenStream {
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < trees.len() {
            if let (
                Some(TokenTree::Ident(this)),
                Some(TokenTree::Punct(dot)),
                Some(TokenTree::Ident(name)),
            ) = (trees.get(i), trees.get(i + 1), trees.get(i + 2))
                && this == "this"
                && dot.as_char() == '.'
                && !matches!(trees.get(i + 3), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis)
                && !matches!(trees.get(i + 3), Some(TokenTree::Punct(p)) if p.as_char() == ':')
            {
                let read = self.read(name, name.span());
                out.push(TokenTree::Group(Group::new(
                    Delimiter::Parenthesis,
                    read.into_token_stream(),
                )));
                i += 3;
                continue;
            }
            match &trees[i] {
                TokenTree::Group(g) => {
                    let mut ng = Group::new(g.delimiter(), self.rewrite_tokens(g.stream()));
                    ng.set_span(g.span());
                    out.push(TokenTree::Group(ng));
                }
                t => out.push(t.clone()),
            }
            i += 1;
        }
        out.into_iter().collect()
    }
}

impl VisitMut for Rewriter<'_> {
    fn visit_expr_mut(&mut self, e: &mut Expr) {
        let span = e.span();
        let replacement: Option<Expr> = match e {
            Expr::Assign(a) => {
                if let Some(name) = this_field(&a.left).cloned() {
                    self.visit_expr_mut(&mut a.right);
                    let right = &a.right;
                    self.cell(&name).map(
                        |cell| parse_quote_spanned!(span=> ::roost::__private::set(#cell, #right)),
                    )
                } else if let Some(name) = place_root_field(&a.left) {
                    let msg = format!(
                        "cannot assign into a part of field `{name}`: `this.{name}` is read as a \
                         copy. Read it into a local, change that, then write it back with \
                         `this.{name} = ...`"
                    );
                    self.error(a.left.span(), msg);
                    None
                } else {
                    visit_mut::visit_expr_assign_mut(self, a);
                    None
                }
            }
            Expr::Binary(b) if compound_to_plain(&b.op) => {
                if let Some(name) = this_field(&b.left).cloned() {
                    self.visit_expr_mut(&mut b.right);
                    let (op, right) = (&b.op, &b.right);
                    // Java order: read the field, evaluate the right side, write back.
                    self.cell(&name).map(|cell| {
                        parse_quote_spanned!(span=> {
                            let mut __roost_value = ::roost::__private::get(#cell);
                            __roost_value #op #right;
                            ::roost::__private::set(#cell, __roost_value)
                        })
                    })
                } else if let Some(name) = place_root_field(&b.left) {
                    let msg = format!(
                        "cannot update a part of field `{name}` in place: read it into a local, \
                         change that, then write it back with `this.{name} = ...`"
                    );
                    self.error(b.left.span(), msg);
                    None
                } else {
                    visit_mut::visit_expr_binary_mut(self, b);
                    None
                }
            }
            Expr::Reference(r) if r.mutability.is_some() && place_root_field(&r.expr).is_some() => {
                self.error(
                    r.span(),
                    "cannot take `&mut` of a field: fields are read as copies",
                );
                None
            }
            Expr::Field(_) if this_field(e).is_some() => {
                let name = this_field(e).cloned().unwrap();
                Some(self.read(&name, span))
            }
            Expr::MethodCall(call) => self.rewrite_method_call(call),
            Expr::Call(c) if is_super(&c.func) => {
                self.error(
                    c.func.span(),
                    "`super(...)` must be the first statement of a constructor",
                );
                None
            }
            Expr::Path(_) if is_super(e) => {
                self.error(span, "`super` can only be used as `super.method(...)`");
                None
            }
            Expr::Macro(m) => {
                let tokens = m.mac.tokens.clone();
                m.mac.tokens = match syn::parse::Parser::parse2(
                    Punctuated::<Expr, Token![,]>::parse_terminated,
                    tokens.clone(),
                ) {
                    Ok(mut args) => {
                        for arg in args.iter_mut() {
                            self.visit_expr_mut(arg);
                        }
                        args.into_token_stream()
                    }
                    Err(_) => self.rewrite_tokens(tokens),
                };
                None
            }
            _ => {
                visit_mut::visit_expr_mut(self, e);
                None
            }
        };
        if let Some(r) = replacement {
            *e = r;
        }
    }

    fn visit_macro_mut(&mut self, m: &mut syn::Macro) {
        // Statement-position macros (`println!(..);`) are `Stmt::Macro`, not `Expr::Macro`.
        let mut e = Expr::Macro(syn::ExprMacro {
            attrs: Vec::new(),
            mac: m.clone(),
        });
        self.visit_expr_mut(&mut e);
        if let Expr::Macro(em) = e {
            *m = em.mac;
        }
    }

    // Nested items (`fn helper() {}` inside a body) have no `this`.
    fn visit_item_mut(&mut self, _: &mut syn::Item) {}
}

/// Splits a leading `super(args);` off a constructor body.
pub fn take_super_call(block: &mut syn::Block) -> Option<(Span, Punctuated<Expr, Token![,]>)> {
    let first = block.stmts.first()?;
    let syn::Stmt::Expr(Expr::Call(call), _) = first else {
        return None;
    };
    if !is_super(&call.func) {
        return None;
    }
    let found = (call.func.span(), call.args.clone());
    block.stmts.remove(0);
    Some(found)
}
