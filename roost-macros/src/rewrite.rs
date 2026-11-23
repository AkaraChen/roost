//! Rewrites method and constructor bodies:
//!
//! | user writes                | becomes                                                   |
//! | -------------------------- | --------------------------------------------------------- |
//! | `this.f` (read)            | `get(field(this.__roost_field_Owner_f(), "Owner", "f"))`: a clone, short borrow |
//! | `this.f.m(..)`             | `ReadOnlyCopyOfField(get(..)).m(..)`: `&mut self` methods don't compile |
//! | `this.f = e`               | `set(field(..), e)`                                       |
//! | `this.f += e` (any op=)    | `{ let mut v = get(..); v += e; set(.., v) }`              |
//! | `super.m(a)`               | `<dyn OwnerClass>::__roost_method_m(this, a)`              |
//! | `this.p(a)`, `p` private   | `<dyn ClassClass>::__roost_private_p(this, a)`             |
//! | `Self::s` / `Class::s`, `s` a static field | `get_static(&__ROOST_STATIC_Owner_s, "Owner", "s")` (and `set_static`) |
//! | `Dog::f(..)`, `f` a static method inherited from `Animal` | `<dyn AnimalClass>::f(..)` |
//!
//! `Owner` is the class that declares `f` / implements `m`, resolved here at expansion time.
//! No `GcCell` guard ever outlives the helper call, so a virtual call can never run while a
//! field is borrowed: the spike's "base class holds `borrow_mut`, override reads the same
//! field" panic cannot happen. Inside macro invocations (`format!(..)`) the arguments are
//! rewritten too when they parse as expressions; otherwise only field reads are.

use proc_macro2::{Delimiter, Group, Span, TokenStream, TokenTree};
use quote::{ToTokens, quote, quote_spanned};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit_mut::{self, VisitMut};
use syn::{BinOp, Expr, ExprMethodCall, Ident, Member, Token, parse_quote_spanned};

use crate::index::{self, Impl, Index};

pub struct Rewriter<'a> {
    pub index: &'a Index,
    pub class: usize,
    /// `false` in static methods.
    pub has_this: bool,
    pub errors: &'a mut Vec<syn::Error>,
}

/// A field as an assignable place.
enum Place {
    /// `this.f`
    This(Ident),
    /// `Class::f` / `Self::f`, a static field of class `of` (or of one of its ancestors).
    Static { of: usize, name: Ident },
}

impl Place {
    fn name(&self) -> &Ident {
        match self {
            Place::This(n) | Place::Static { name: n, .. } => n,
        }
    }

    /// How the user wrote it, for messages.
    fn display(&self, index: &Index) -> String {
        match self {
            Place::This(n) => format!("this.{n}"),
            Place::Static { of, name } => format!("{}::{name}", index.classes[*of].name),
        }
    }
}

/// A resolved field: read with `get(arg)`, write with `set(arg, value)`.
struct Cell {
    get: TokenStream,
    set: TokenStream,
    arg: TokenStream,
}

/// `this.f` -> `f`.
fn this_field(e: &Expr) -> Option<&Ident> {
    let Expr::Field(f) = e else { return None };
    let Member::Named(name) = &f.member else {
        return None;
    };
    is_path(&f.base, "this").then_some(name)
}

fn is_path(e: &Expr, name: &str) -> bool {
    matches!(e, Expr::Path(p) if p.qself.is_none() && p.path.is_ident(name))
}

/// `super` as a single-segment path. (`Path::is_ident` is false for keywords.)
fn is_super(e: &Expr) -> bool {
    is_keyword(e, "super")
}

/// `e` is the single-segment path `word`, which may be a keyword (`self`, `super`).
fn is_keyword(e: &Expr, word: &str) -> bool {
    match e {
        Expr::Path(p) => {
            p.qself.is_none()
                && p.path.leading_colon.is_none()
                && p.path.segments.len() == 1
                && p.path.segments[0].ident == word
                && p.path.segments[0].arguments.is_none()
        }
        _ => false,
    }
}

/// `A::b` (no generics, no leading `::`) -> `(A, b)`.
fn two_segment_path(e: &Expr) -> Option<(&Ident, &Ident)> {
    let Expr::Path(p) = e else { return None };
    let segs = &p.path.segments;
    (p.qself.is_none()
        && p.path.leading_colon.is_none()
        && segs.len() == 2
        && segs.iter().all(|s| s.arguments.is_none()))
    .then(|| (&segs[0].ident, &segs[1].ident))
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
    /// `msg` may carry a help line after [`index::HELP`].
    fn error(&mut self, span: Span, msg: impl std::fmt::Display) {
        self.errors.push(index::error(span, &msg.to_string()));
    }

    /// The class a path's first segment names: `Self` or a class of this block.
    fn path_class(&self, first: &Ident) -> Option<usize> {
        if first == "Self" {
            Some(self.class)
        } else {
            self.index
                .find(first)
                .filter(|&c| !self.index.classes[c].is_interface())
        }
    }

    /// `this.f`, or `Class::f` / `Self::f` naming a static field.
    fn place(&self, e: &Expr) -> Option<Place> {
        if let Some(name) = this_field(e) {
            return Some(Place::This(name.clone()));
        }
        let (first, name) = two_segment_path(e)?;
        let of = self.path_class(first)?;
        let _found = self.index.resolve_static_field(self.class, of, name)?;
        Some(Place::Static {
            of,
            name: name.clone(),
        })
    }

    /// The root of a place expression (`this.f.g[0]` -> `this.f`), if it is a field.
    fn place_root(&self, e: &Expr) -> Option<Place> {
        if let Some(p) = self.place(e) {
            return Some(p);
        }
        match e {
            Expr::Field(f) => self.place_root(&f.base),
            Expr::Index(i) => self.place_root(&i.expr),
            Expr::Paren(p) => self.place_root(&p.expr),
            Expr::Unary(u) => self.place_root(&u.expr),
            _ => None,
        }
    }

    /// How to read and write a place. `None` (error recorded) if it doesn't resolve.
    fn cell(&mut self, place: &Place) -> Option<Cell> {
        let name = place.name();
        match place {
            Place::This(_) => {
                if !self.has_this {
                    self.error(
                        name.span(),
                        format!(
                            "there is no `this` in a static method{}a static method belongs \
                             to the class, not to an object (as in Java): remove `static`, or \
                             pass the object in as a parameter",
                            index::HELP
                        ),
                    );
                    return None;
                }
                match self.index.resolve_field(self.class, name) {
                    Ok(f) => {
                        // The names are for runtime error messages (borrow conflicts).
                        let owner = &self.index.classes[f.owner];
                        let accessor = index::accessor_name(owner, name);
                        let accessor = Ident::new(&accessor.to_string(), name.span());
                        let (class, field) = (owner.name.to_string(), name.to_string());
                        Some(Cell {
                            get: quote_spanned!(name.span()=> ::roost::__private::get),
                            set: quote_spanned!(name.span()=> ::roost::__private::set),
                            arg: quote_spanned!(name.span()=>
                                ::roost::__private::field(this.#accessor(), #class, #field)),
                        })
                    }
                    Err(msg) => {
                        self.error(name.span(), msg);
                        None
                    }
                }
            }
            Place::Static { of, .. } => {
                match self.index.resolve_static_field(self.class, *of, name)? {
                    Ok(f) => {
                        let owner = &self.index.classes[f.owner];
                        let key = index::static_name(owner, name);
                        let key = Ident::new(&key.to_string(), name.span());
                        let (class, field) = (owner.name.to_string(), name.to_string());
                        Some(Cell {
                            get: quote_spanned!(name.span()=> ::roost::__private::get_static),
                            set: quote_spanned!(name.span()=> ::roost::__private::set_static),
                            arg: quote_spanned!(name.span()=> &#key, #class, #field),
                        })
                    }
                    Err(msg) => {
                        self.error(name.span(), msg);
                        None
                    }
                }
            }
        }
    }

    fn read(&mut self, place: &Place, span: Span) -> Expr {
        match self.cell(place) {
            Some(Cell { get, arg, .. }) => parse_quote_spanned!(span=> #get(#arg)),
            None => parse_quote_spanned!(span=> ::core::unreachable!()),
        }
    }

    fn visit_args(&mut self, call: &mut ExprMethodCall) {
        for arg in call.args.iter_mut() {
            self.visit_expr_mut(arg);
        }
    }

    fn super_call(&mut self, call: &mut ExprMethodCall) -> Option<Expr> {
        let span = call.span();
        let method = call.method.clone();
        self.visit_args(call);
        let is_object_method = index::OBJECT_METHODS.iter().any(|o| method == o);
        let resolved = match self.index.parent(self.class) {
            Some(parent) => self.index.resolve_impl(parent, &method),
            // The superclass is `Object`.
            None if is_object_method && !self.index.classes[self.class].is_interface() => {
                Some(Impl::Object)
            }
            None => {
                self.error(
                    call.receiver.span(),
                    "`super` method call in a class without a superclass",
                );
                return None;
            }
        };
        let f = index::method_impl_name(&method);
        let f = Ident::new(&f.to_string(), method.span());
        let args = &call.args;
        let tr = |t: usize| index::trait_name(&self.index.classes[t]);
        Some(match resolved {
            Some(Impl::Class(k) | Impl::Default(k)) => {
                let tr = tr(k);
                parse_quote_spanned!(span=> <dyn #tr>::#f(this, #args))
            }
            Some(Impl::Object) => {
                let default = match method.to_string().as_str() {
                    "to_string" => quote!(default_to_string),
                    "hash_code" => quote!(identity_hash_code),
                    _ => quote!(identity_equals),
                };
                let args = args.iter().map(|a| quote!(&#a));
                parse_quote_spanned!(span=> ::roost::__private::#default(this #(, #args)*))
            }
            None => {
                let parent = self.index.parent(self.class).expect("handled above");
                let msg = match self.index.resolve_virtual(parent, &method) {
                    Some(owner) => format!(
                        "`{}.{method}` is abstract: it cannot be called with `super`",
                        self.index.classes[owner].name
                    ),
                    None => format!("no method `{method}` in any superclass"),
                };
                self.error(method.span(), msg);
                return None;
            }
        })
    }

    fn rewrite_method_call(&mut self, call: &mut ExprMethodCall) -> Option<Expr> {
        let span = call.span();
        let method = call.method.clone();
        if is_super(&call.receiver) {
            return self.super_call(call);
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
        if let Some(place) = self.place(&call.receiver) {
            // A method called on a field operates on a *copy*. Make the copy immutable, so a
            // `&mut self` method (`this.items.push(x)`) is a compile error instead of a
            // silently lost update.
            self.visit_args(call);
            let rspan = call.receiver.span();
            let read = self.read(&place, rspan);
            *call.receiver =
                parse_quote_spanned!(rspan=> ::roost::__private::ReadOnlyCopyOfField(#read));
            return None;
        }
        visit_mut::visit_expr_method_call_mut(self, call);
        None
    }

    /// `Dog::f` where `f` is a static method `Dog` inherits from `Animal`: the inherent
    /// function lives on `dyn AnimalClass`, so name that.
    fn inherited_static(&self, e: &Expr) -> Option<Expr> {
        let (first, name) = two_segment_path(e)?;
        let of = self.path_class(first)?;
        let owner = self.index.resolve_static_method(of, name)?;
        if owner == of {
            return None;
        }
        let tr = index::trait_name(&self.index.classes[owner]);
        Some(parse_quote_spanned!(e.span()=> <dyn #tr>::#name))
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
                let read = self.read(&Place::This(name.clone()), name.span());
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
                if let Some(place) = self.place(&a.left) {
                    self.visit_expr_mut(&mut a.right);
                    let right = &a.right;
                    self.cell(&place).map(
                        |Cell { set, arg, .. }| parse_quote_spanned!(span=> #set(#arg, #right)),
                    )
                } else if let Some(place) = self.place_root(&a.left) {
                    let p = place.display(self.index);
                    let msg = format!(
                        "cannot assign into a part of field `{}`: `{p}` is read as a copy. Read \
                         it into a local, change that, then write it back with `{p} = ...`",
                        place.name()
                    );
                    self.error(a.left.span(), msg);
                    None
                } else {
                    visit_mut::visit_expr_assign_mut(self, a);
                    None
                }
            }
            Expr::Binary(b) if compound_to_plain(&b.op) => {
                if let Some(place) = self.place(&b.left) {
                    self.visit_expr_mut(&mut b.right);
                    let (op, right) = (&b.op, &b.right);
                    // Java order: read the field, evaluate the right side, write back.
                    self.cell(&place).map(|Cell { get, set, arg }| {
                        parse_quote_spanned!(span=> {
                            let mut __roost_value = #get(#arg);
                            __roost_value #op #right;
                            #set(#arg, __roost_value)
                        })
                    })
                } else if let Some(place) = self.place_root(&b.left) {
                    let p = place.display(self.index);
                    let msg = format!(
                        "cannot update a part of field `{}` in place: read it into a local, \
                         change that, then write it back with `{p} = ...`",
                        place.name()
                    );
                    self.error(b.left.span(), msg);
                    None
                } else {
                    visit_mut::visit_expr_binary_mut(self, b);
                    None
                }
            }
            Expr::Reference(r) if r.mutability.is_some() && self.place_root(&r.expr).is_some() => {
                self.error(
                    r.span(),
                    "cannot take `&mut` of a field: fields are read as copies",
                );
                None
            }
            Expr::Call(c) if two_segment_path(&c.func).is_some() => {
                // `Class::f(..)` calls a static method, even if a static field is also
                // called `f` (Java keeps fields and methods apart, so do we).
                if let Some(func) = self.inherited_static(&c.func) {
                    *c.func = func;
                }
                for arg in c.args.iter_mut() {
                    self.visit_expr_mut(arg);
                }
                None
            }
            Expr::Field(_) | Expr::Path(_) if self.place(e).is_some() => {
                let place = self.place(e).unwrap();
                Some(self.read(&place, span))
            }
            Expr::Path(_) if self.inherited_static(e).is_some() => self.inherited_static(e),
            Expr::MethodCall(call) => self.rewrite_method_call(call),
            Expr::Call(c) if is_super(&c.func) => {
                self.error(
                    c.func.span(),
                    format!(
                        "`super(...)` must be the first statement of a constructor{}move it to \
                         the top of the constructor body, as in Java",
                        index::HELP
                    ),
                );
                None
            }
            Expr::Path(_) if is_keyword(e, "self") => {
                self.error(
                    span,
                    format!(
                        "use `this`, not `self`, in a class{}roost methods are written like \
                         Java's: `this.field`, `this.method()`",
                        index::HELP
                    ),
                );
                None
            }
            Expr::Path(_) if is_keyword(e, "null") => {
                self.error(
                    span,
                    format!(
                        "there is no `null` in roost (nor in Rust){}a reference that can be \
                         missing is an `Option<Ref<..>>` field: write `None` here, and \
                         `Some(x)` for a value",
                        index::HELP
                    ),
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
