//! The class DSL's syntax tree, and its parser.
//!
//! ```text
//! input   := class*
//! class   := attr* vis "class" Ident ("extends" Ident)? "{" member* "}"
//! member  := attr* ("@" "Override")? vis ( field | ctor | method )
//! field   := Ident ":" Type ("=" Expr)? ";"
//! ctor    := Ident "(" params ")" Block            -- Ident is the class name
//! method  := "static"? "fn" Ident "(" params ")" ("->" Type)? Block
//! vis     := "public" | "protected" | "private" | ε   -- ε is Java's package-private
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
    syn::custom_keyword!(extends);
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

pub struct Hierarchy {
    pub classes: Vec<Class>,
}

pub struct Class {
    pub attrs: Vec<Attribute>,
    pub access: Access,
    pub name: Ident,
    pub parent: Option<Ident>,
    pub fields: Vec<Field>,
    pub ctor: Option<Ctor>,
    pub methods: Vec<Method>,
}

pub struct Field {
    pub attrs: Vec<Attribute>,
    pub access: Access,
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
    pub sig: Signature,
    pub body: Block,
}

impl Method {
    pub fn name(&self) -> &Ident {
        &self.sig.ident
    }

    /// Virtual (goes into the class trait's vtable): not static, not private.
    pub fn is_virtual(&self) -> bool {
        !self.is_static && self.access != Access::Private
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

impl Parse for Class {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let access = parse_access(input)?;
        if access == Access::Private || access == Access::Protected {
            return Err(input.error("a top-level class is `public` or package-private"));
        }
        for unsupported in ["abstract", "final", "interface"] {
            if input.peek(Ident) && input.fork().parse::<Ident>()? == unsupported {
                return Err(input.error(format!("`{unsupported}` is not supported yet")));
            }
        }
        input.parse::<kw::class>()?;
        let name: Ident = input.parse()?;
        let parent = if input.parse::<Option<kw::extends>>()?.is_some() {
            Some(input.parse()?)
        } else {
            None
        };
        if input.peek(Token![<]) {
            return Err(input.error("generic classes are not supported"));
        }
        let content;
        braced!(content in input);
        let mut class = Class {
            attrs,
            access,
            name,
            parent,
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

    // Method.
    let is_static = input.parse::<Option<Token![static]>>()?.is_some();
    if is_static || input.peek(Token![fn]) {
        let sig: Signature = input.parse()?;
        check_signature(&sig)?;
        let body: Block = input.parse()?;
        if is_static && override_.is_some() {
            return Err(syn::Error::new(
                sig.ident.span(),
                "a static method cannot be @Override",
            ));
        }
        class.methods.push(Method {
            attrs,
            override_,
            access,
            is_static,
            sig,
            body,
        });
        return Ok(());
    }

    let name: Ident = input.parse()?;
    if let Some(span) = override_ {
        return Err(syn::Error::new(span, "@Override only applies to methods"));
    }

    // Constructor.
    if name == class.name {
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
