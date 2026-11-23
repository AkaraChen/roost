//! `new!(Dog("Rex"))` => `<Dog>::new("Rex")`.
//!
//! The call is spanned at the user's tokens (`Dog` and the parentheses), so a wrong argument
//! count or type is reported at `Dog(..)`, and the Java habits (`new!(new Dog(..))`,
//! `new!(Dog)`, `new!(Dog::new(..))`) get an error saying what to write instead.

use proc_macro2::{Group, TokenStream, TokenTree};
use quote::{quote, quote_spanned};
use syn::parse::{ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Expr, Path, Token};

use crate::diag::{self, Help};

const USAGE: &str = "write `new!(ClassName(arguments..))`, e.g. `new!(Dog(\"Rex\"))`";

pub fn expand(input: TokenStream) -> TokenStream {
    match parse.parse2(input) {
        Ok(out) => out,
        Err(e) => {
            let errors = diag::emit([&e]);
            // An expression of any type, so that no follow-up error piles on.
            quote!({ #errors ::core::unreachable!() })
        }
    }
}

fn parse(input: ParseStream) -> syn::Result<TokenStream> {
    if input.is_empty() {
        return Err(input
            .error("`new!` needs a class and its constructor arguments")
            .help(USAGE));
    }
    if input.peek(syn::Ident)
        && input.fork().parse::<syn::Ident>()? == "new"
        && input.peek2(syn::Ident)
    {
        let new: syn::Ident = input.parse()?;
        return Err(syn::Error::new(
            new.span(),
            "`new!` already means `new`: don't write `new` again inside it",
        )
        .help("Java's `new Dog(\"Rex\")` is `new!(Dog(\"Rex\"))`"));
    }
    let path = Path::parse_mod_style(input)?;
    if let Some(last) = path.segments.last()
        && last.ident == "new"
        && path.segments.len() > 1
    {
        return Err(syn::Error::new(
            last.ident.span(),
            "`new!` calls the constructor `::new` itself",
        )
        .help("write `new!(Dog(\"Rex\"))`, or call `Dog::new(\"Rex\")` without `new!`"));
    }
    if !input.peek(syn::token::Paren) {
        let class = quote!(#path).to_string().replace(' ', "");
        return Err(syn::Error::new(
            path.span(),
            format!("missing the constructor arguments of `{class}`"),
        )
        .help(format!(
            "write `new!({class}())`: like Java's `new {class}()`, the parentheses are required \
             even without arguments"
        )));
    }
    let parens = match input.parse::<TokenTree>()? {
        TokenTree::Group(g) => g,
        _ => unreachable!("peeked a parenthesized group"),
    };
    let args = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(parens.stream())?;
    if !input.is_empty() {
        return Err(input
            .error("unexpected tokens after the constructor arguments")
            .help(USAGE));
    }
    let args = args.into_iter();
    let mut call = Group::new(proc_macro2::Delimiter::Parenthesis, quote!(#(#args),*));
    call.set_span(parens.span());
    let span = path.span();
    let new = syn::Ident::new("new", span);
    Ok(quote_spanned!(span=> <#path>::#new #call))
}
