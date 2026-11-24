//! The class DSL's syntax tree, and its parser.
//!
//! ```text
//! input     := (class | interface)*
//! class     := attr* vis ("abstract" | "final")? "class" Ident ("extends" Ident)?
//!              ("implements" Ident ("," Ident)*)? "{" member* "}"
//! interface := attr* vis "interface" Ident ("extends" Ident ("," Ident)*)? "{" imember* "}"
//! member    := attr* ("@" "Override")? vis modifier* ( field | ctor | method )
//! modifier  := "static" | "abstract" | "final"
//! field     := Ident ":" Type ("=" Expr)? ";"
//! ctor      := Ident "(" params ")" Block            -- Ident is the class name
//! method    := "fn" Ident "(" params ")" ("->" Type)? (Block | ";")   -- ";" iff abstract
//! imember   := attr* "public"? ( "fn" sig ";" | "default" "fn" sig Block | "static" "fn" sig Block )
//! vis       := "public" | "protected" | "private" | ε   -- ε is Java's package-private
//! ```

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Block, Expr, FnArg, Ident, PatType, Signature, Token, Type, braced};

mod kw {
    syn::custom_keyword!(public);
    syn::custom_keyword!(protected);
    syn::custom_keyword!(private);
    syn::custom_keyword!(class);
    syn::custom_keyword!(interface);
    syn::custom_keyword!(extends);
    syn::custom_keyword!(implements);
}

/// Java access modifiers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    Public,
    Protected,
    /// No modifier: Java's package-private.
    Package,
    Private,
}

impl Access {
    /// The closest Rust visibility. Java packages map to the crate.
    pub fn to_rust(self) -> TokenStream {
        match self {
            Access::Public => quote!(pub),
            Access::Protected | Access::Package => quote!(pub(crate)),
            Access::Private => quote!(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Class,
    Interface,
}

pub struct Hierarchy {
    pub classes: Vec<Class>,
}

/// A class or an interface. Interfaces have no fields, no constructor and no superclass;
/// their super-interfaces are in `interfaces`.
pub struct Class {
    pub attrs: Vec<Attribute>,
    pub access: Access,
    pub kind: Kind,
    /// Span of `abstract` / `final`, if present.
    pub abstract_: Option<Span>,
    pub final_: Option<Span>,
    pub name: Ident,
    pub parent: Option<Ident>,
    /// `implements` (class) or `extends` (interface) list.
    pub interfaces: Vec<Ident>,
    pub fields: Vec<Field>,
    pub ctor: Option<Ctor>,
    pub methods: Vec<Method>,
}

impl Class {
    pub fn is_interface(&self) -> bool {
        self.kind == Kind::Interface
    }

    pub fn instance_fields(&self) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(|f| !f.is_static)
    }

    pub fn static_fields(&self) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(|f| f.is_static)
    }
}

pub struct Field {
    pub attrs: Vec<Attribute>,
    pub access: Access,
    pub is_static: bool,
    pub name: Ident,
    pub ty: Type,
    pub init: Option<Expr>,
}

pub struct Ctor {
    pub attrs: Vec<Attribute>,
    pub access: Access,
    pub name: Ident,
    pub params: Vec<PatType>,
    pub body: Block,
}

pub struct Method {
    pub attrs: Vec<Attribute>,
    /// Span of `@Override`, if present.
    pub override_: Option<Span>,
    pub access: Access,
    pub is_static: bool,
    /// Span of `abstract` / `final`, if present.
    pub abstract_: Option<Span>,
    pub final_: Option<Span>,
    pub sig: Signature,
    /// `None` for abstract methods (and interface methods without `default`).
    pub body: Option<Block>,
}

impl Method {
    pub fn name(&self) -> &Ident {
        &self.sig.ident
    }

    /// Virtual (goes into a vtable): not static, not private.
    pub fn is_virtual(&self) -> bool {
        !self.is_static && self.access != Access::Private
    }

    /// Declared without a body (an abstract method, or an interface method without
    /// `default`).
    pub fn is_abstract(&self) -> bool {
        self.body.is_none()
    }

    pub fn params(&self) -> impl Iterator<Item = &PatType> {
        self.sig.inputs.iter().map(|arg| match arg {
            FnArg::Typed(pt) => pt,
            FnArg::Receiver(_) => unreachable!("rejected by the parser"),
        })
    }
}

impl Parse for Hierarchy {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut classes = Vec::new();
        while !input.is_empty() {
            classes.push(input.parse()?);
        }
        Ok(Hierarchy { classes })
    }
}

fn parse_access(input: ParseStream) -> syn::Result<Access> {
    if input.peek(Token![pub]) {
        return Err(
            input.error("write `public`, `protected` or `private` (Java modifiers), not `pub`")
        );
    }
    Ok(if input.parse::<Option<kw::public>>()?.is_some() {
        Access::Public
    } else if input.parse::<Option<kw::protected>>()?.is_some() {
        Access::Protected
    } else if input.parse::<Option<kw::private>>()?.is_some() {
        Access::Private
    } else {
        Access::Package
    })
}

fn parse_name_list(input: ParseStream) -> syn::Result<Vec<Ident>> {
    let mut names = vec![input.parse::<Ident>()?];
    while input.parse::<Option<Token![,]>>()?.is_some() {
        names.push(input.parse()?);
    }
    Ok(names)
}

/// `static` / `abstract` / `final`, in any order, each at most once.
#[derive(Default)]
struct Modifiers {
    static_: Option<Span>,
    abstract_: Option<Span>,
    final_: Option<Span>,
}

fn parse_modifiers(input: ParseStream) -> syn::Result<Modifiers> {
    let mut m = Modifiers::default();
    loop {
        let (slot, span) = if let Some(t) = input.parse::<Option<Token![static]>>()? {
            (&mut m.static_, t.span)
        } else if let Some(t) = input.parse::<Option<Token![abstract]>>()? {
            (&mut m.abstract_, t.span)
        } else if let Some(t) = input.parse::<Option<Token![final]>>()? {
            (&mut m.final_, t.span)
        } else {
            return Ok(m);
        };
        if slot.replace(span).is_some() {
            return Err(syn::Error::new(span, "repeated modifier"));
        }
    }
}

impl Parse for Class {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let access = parse_access(input)?;
        if access == Access::Private || access == Access::Protected {
            return Err(input.error("a top-level class is `public` or package-private"));
        }
        let mods = parse_modifiers(input)?;
        if let Some(s) = mods.static_ {
            return Err(syn::Error::new(s, "a top-level class cannot be `static`"));
        }
        if let (Some(_), Some(f)) = (mods.abstract_, mods.final_) {
            return Err(syn::Error::new(
                f,
                "a class cannot be both `abstract` and `final`",
            ));
        }
        if input.peek(kw::interface) {
            input.parse::<kw::interface>()?;
            if let Some(s) = mods.abstract_.or(mods.final_) {
                return Err(syn::Error::new(
                    s,
                    "an interface cannot be `abstract` or `final` (it is always abstract)",
                ));
            }
            return parse_interface(input, attrs, access);
        }
        input.parse::<kw::class>()?;
        let name: Ident = input.parse()?;
        if input.peek(Token![<]) {
            return Err(input.error(
                "generic classes are not supported (see the README's list of limitations)",
            ));
        }
        let parent = if input.parse::<Option<kw::extends>>()?.is_some() {
            Some(input.parse()?)
        } else {
            None
        };
        let interfaces = if input.parse::<Option<kw::implements>>()?.is_some() {
            parse_name_list(input)?
        } else {
            Vec::new()
        };
        let content;
        braced!(content in input);
        let mut class = Class {
            attrs,
            access,
            kind: Kind::Class,
            abstract_: mods.abstract_,
            final_: mods.final_,
            name,
            parent,
            interfaces,
            fields: Vec::new(),
            ctor: None,
            methods: Vec::new(),
        };
        while !content.is_empty() {
            parse_member(&content, &mut class)?;
        }
        Ok(class)
    }
}

fn parse_interface(
    input: ParseStream,
    attrs: Vec<Attribute>,
    access: Access,
) -> syn::Result<Class> {
    let name: Ident = input.parse()?;
    if input.peek(Token![<]) {
        return Err(input.error("generic interfaces are not supported"));
    }
    if input.peek(kw::implements) {
        return Err(
            input.error("an interface `extends` other interfaces, it doesn't `implements` them")
        );
    }
    let interfaces = if input.parse::<Option<kw::extends>>()?.is_some() {
        parse_name_list(input)?
    } else {
        Vec::new()
    };
    let content;
    braced!(content in input);
    let mut methods = Vec::new();
    while !content.is_empty() {
        methods.push(parse_interface_member(&content)?);
    }
    Ok(Class {
        attrs,
        access,
        kind: Kind::Interface,
        abstract_: None,
        final_: None,
        name,
        parent: None,
        interfaces,
        fields: Vec::new(),
        ctor: None,
        methods,
    })
}

fn parse_interface_member(input: ParseStream) -> syn::Result<Method> {
    let attrs = input.call(Attribute::parse_outer)?;
    if input.peek(Token![@]) {
        return Err(input.error(
            "annotations are not supported in interfaces (an interface method cannot \
             re-declare a super-interface's method)",
        ));
    }
    match parse_access(input)? {
        Access::Public | Access::Package => {}
        _ => {
            return Err(input.error(
                "interface methods are always public: `private` / `protected` are not supported",
            ));
        }
    }
    let default = input.parse::<Option<Token![default]>>()?;
    let mods = parse_modifiers(input)?;
    if let Some(s) = mods.abstract_.or(mods.final_) {
        return Err(syn::Error::new(
            s,
            "interface methods are implicitly abstract (no body) or `default` (body); \
             `abstract` / `final` are not allowed",
        ));
    }
    if !input.peek(Token![fn]) {
        return Err(input.error(
            "expected a method: an interface has no fields or constructors \
             (static fields are not supported in interfaces)",
        ));
    }
    let sig: Signature = input.parse()?;
    check_signature(&sig)?;
    let is_static = mods.static_.is_some();
    let body = if input.peek(Token![;]) {
        let semi: Token![;] = input.parse()?;
        if let Some(d) = &default {
            return Err(syn::Error::new(d.span, "a `default` method needs a body"));
        }
        if is_static {
            return Err(syn::Error::new(semi.span, "a static method needs a body"));
        }
        None
    } else {
        let block: Block = input.parse()?;
        if default.is_none() && !is_static {
            return Err(syn::Error::new(
                sig.ident.span(),
                "an interface method with a body must be `default` (or `static`)",
            ));
        }
        if let (Some(d), true) = (&default, is_static) {
            return Err(syn::Error::new(
                d.span,
                "a static method cannot be `default`",
            ));
        }
        Some(block)
    };
    Ok(Method {
        attrs,
        override_: None,
        access: Access::Public,
        is_static,
        abstract_: None,
        final_: None,
        sig,
        body,
    })
}

fn parse_member(input: ParseStream, class: &mut Class) -> syn::Result<()> {
    let attrs = input.call(Attribute::parse_outer)?;
    let override_ = if input.peek(Token![@]) {
        let at: Token![@] = input.parse()?;
        let name: Ident = input.parse()?;
        if name != "Override" {
            return Err(syn::Error::new(
                name.span(),
                "unknown annotation: only `@Override` is supported",
            ));
        }
        Some(at.span.join(name.span()).unwrap_or(name.span()))
    } else {
        None
    };
    let access = parse_access(input)?;
    let mods = parse_modifiers(input)?;
    let is_static = mods.static_.is_some();

    // Method.
    if input.peek(Token![fn]) {
        let sig: Signature = input.parse()?;
        check_signature(&sig)?;
        let body = if input.peek(Token![;]) {
            let semi: Token![;] = input.parse()?;
            if mods.abstract_.is_none() {
                return Err(syn::Error::new(
                    semi.span,
                    "a method without a body must be declared `abstract`",
                ));
            }
            None
        } else {
            let block: Block = input.parse()?;
            if let Some(a) = mods.abstract_ {
                return Err(syn::Error::new(
                    a,
                    "an `abstract` method cannot have a body",
                ));
            }
            Some(block)
        };
        if is_static && override_.is_some() {
            return Err(syn::Error::new(
                sig.ident.span(),
                "a static method cannot be @Override",
            ));
        }
        if let Some(a) = mods.abstract_ {
            if is_static || access == Access::Private {
                return Err(syn::Error::new(
                    a,
                    "an abstract method cannot be `static` or `private` (it must be overridable)",
                ));
            }
            if mods.final_.is_some() {
                return Err(syn::Error::new(
                    a,
                    "a method cannot be both `abstract` and `final`",
                ));
            }
        }
        class.methods.push(Method {
            attrs,
            override_,
            access,
            is_static,
            abstract_: mods.abstract_,
            final_: mods.final_,
            sig,
            body,
        });
        return Ok(());
    }

    let name: Ident = input.parse()?;
    if let Some(span) = override_ {
        return Err(syn::Error::new(span, "@Override only applies to methods"));
    }
    if let Some(s) = mods.abstract_ {
        return Err(syn::Error::new(s, "only methods can be `abstract`"));
    }
    if let Some(s) = mods.final_ {
        return Err(syn::Error::new(
            s,
            "`final` fields and constructors are not supported (only `final` classes and methods)",
        ));
    }

    // Constructor.
    if name == class.name {
        if let Some(s) = mods.static_ {
            return Err(syn::Error::new(s, "a constructor cannot be `static`"));
        }
        let content;
        syn::parenthesized!(content in input);
        let params = Punctuated::<PatType, Token![,]>::parse_terminated_with(&content, |p| {
            match p.parse::<FnArg>()? {
                FnArg::Typed(pt) => Ok(pt),
                FnArg::Receiver(r) => Err(syn::Error::new(
                    r.span(),
                    "constructors take no `self`; use `this` in the body",
                )),
            }
        })?;
        let body: Block = input.parse()?;
        if class.ctor.is_some() {
            return Err(syn::Error::new(
                name.span(),
                "only one constructor per class is supported (no overloading in Rust)",
            ));
        }
        class.ctor = Some(Ctor {
            attrs,
            access,
            name,
            params: params.into_iter().collect(),
            body,
        });
        return Ok(());
    }

    // Field.
    if input.peek(syn::token::Paren) {
        return Err(syn::Error::new(
            name.span(),
            format!(
                "a constructor must be named after its class `{}`; a method needs `fn`",
                class.name
            ),
        ));
    }
    input.parse::<Token![:]>()?;
    let ty: Type = input.parse()?;
    let init = if input.parse::<Option<Token![=]>>()?.is_some() {
        Some(input.parse()?)
    } else {
        None
    };
    input.parse::<Token![;]>()?;
    class.fields.push(Field {
        attrs,
        access,
        is_static,
        name,
        ty,
        init,
    });
    Ok(())
}

fn check_signature(sig: &Signature) -> syn::Result<()> {
    if let Some(r) = sig.receiver() {
        return Err(syn::Error::new(
            r.span(),
            "methods take no `self` parameter: `this` is implicit",
        ));
    }
    if !sig.generics.params.is_empty() || sig.generics.where_clause.is_some() {
        return Err(syn::Error::new(
            sig.generics.span(),
            "generic methods are not supported (they cannot be virtual)",
        ));
    }
    if let Some(t) = &sig.asyncness {
        return Err(syn::Error::new(t.span, "async methods are not supported"));
    }
    if let Some(t) = &sig.constness {
        return Err(syn::Error::new(t.span, "const methods are not supported"));
    }
    if let Some(t) = &sig.unsafety {
        return Err(syn::Error::new(t.span, "unsafe methods are not supported"));
    }
    if let Some(t) = &sig.abi {
        return Err(syn::Error::new(
            t.span(),
            "extern methods are not supported",
        ));
    }
    if let Some(v) = &sig.variadic {
        return Err(syn::Error::new(
            v.span(),
            "variadic methods are not supported",
        ));
    }
    Ok(())
}
