//! `#[roost::main]`: wraps `fn main` in the runtime shell (`roost::runtime::run`). The body
//! is not touched: it is moved, as written, into an inner function.
//!
//! ```text
//! #[roost::main(stack = "64MB")]          fn main() {
//! fn main() -> Result<(), E> { body }  =>     fn __roost_main() -> Result<(), E> { body }
//!                                            ::roost::runtime::run(Config { .. }, || match __roost_main() {
//!                                                Ok(()) => 0,
//!                                                Err(e) => { report(e); 1 }
//!                                            })
//!                                        }
//! ```

use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{ItemFn, LitInt, LitStr, ReturnType, Type};

use crate::diag::Help;

#[derive(Default)]
struct Args {
    stack: Option<usize>,
    gc_threshold: Option<usize>,
    gc_stats: bool,
}

const USAGE: &str = "expected `stack = \"64MB\"`, `gc_threshold = \"1MB\"` or `gc_stats`";

/// `"64MB"`, `"512 KiB"`, `"1048576"`, or an integer literal: a size in bytes.
fn parse_size(input: syn::parse::ParseStream<'_>) -> syn::Result<usize> {
    let (text, span) = if input.peek(LitInt) {
        let lit: LitInt = input.parse()?;
        (lit.base10_digits().to_string(), lit.span())
    } else {
        let lit: LitStr = input
            .parse()
            .map_err(|e| syn::Error::new(e.span(), "expected a size such as \"64MB\""))?;
        (lit.value(), lit.span())
    };
    size_in_bytes(&text).map_err(|msg| syn::Error::new(span, msg))
}

fn size_in_bytes(text: &str) -> Result<usize, String> {
    let t = text.trim();
    let digits_end = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let (num, unit) = t.split_at(digits_end);
    let bad = || {
        format!(
            "invalid size {text:?}: write a number of bytes or a number with a unit, e.g. \"64MB\" (B, KB, MB, GB; powers of 1024)"
        )
    };
    let n: usize = num.parse().map_err(|_| bad())?;
    let shift = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 0,
        "k" | "kb" | "kib" => 10,
        "m" | "mb" | "mib" => 20,
        "g" | "gb" | "gib" => 30,
        _ => return Err(bad()),
    };
    let bytes = n
        .checked_mul(1 << shift)
        .ok_or_else(|| format!("size {text:?} is too large"))?;
    if bytes == 0 {
        return Err(format!("size {text:?} must not be zero"));
    }
    Ok(bytes)
}

fn parse_args(attr: TokenStream) -> syn::Result<Args> {
    let mut args = Args::default();
    let parser = syn::meta::parser(|meta| {
        let set_once = |seen: bool| {
            if seen {
                Err(meta.error("this argument is given twice"))
            } else {
                Ok(())
            }
        };
        if meta.path.is_ident("stack") {
            set_once(args.stack.is_some())?;
            args.stack = Some(parse_size(meta.value()?)?);
        } else if meta.path.is_ident("gc_threshold") {
            set_once(args.gc_threshold.is_some())?;
            args.gc_threshold = Some(parse_size(meta.value()?)?);
        } else if meta.path.is_ident("gc_stats") {
            set_once(args.gc_stats)?;
            if !meta.input.is_empty() && !meta.input.peek(syn::Token![,]) {
                return Err(meta.error("`gc_stats` takes no value: write just `gc_stats`"));
            }
            args.gc_stats = true;
        } else {
            let ident = meta.path.get_ident().map(ToString::to_string);
            let name = ident
                .as_deref()
                .map_or_else(|| "this".to_string(), |i| format!("`{i}`"));
            let e = meta.error(format!("unknown argument {name}"));
            return Err(
                match ident
                    .as_deref()
                    .and_then(|i| crate::diag::similar(i, ["stack", "gc_threshold", "gc_stats"]))
                {
                    Some(k) => e.help(format!("did you mean `{k}`?")),
                    None => e.help(USAGE),
                },
            );
        }
        Ok(())
    });
    syn::parse::Parser::parse2(parser, attr)?;
    Ok(args)
}

fn is_unit(ret: &ReturnType) -> bool {
    match ret {
        ReturnType::Default => true,
        ReturnType::Type(_, ty) => matches!(&**ty, Type::Tuple(t) if t.elems.is_empty()),
    }
}

fn check_signature(f: &ItemFn) -> syn::Result<()> {
    let sig = &f.sig;
    let err = |span: Span, msg: &str| Err(syn::Error::new(span, msg));
    if sig.ident != "main" {
        return Err(syn::Error::new(
            sig.ident.span(),
            "`#[roost::main]` goes on the program's `fn main`, not on other functions",
        )
        .help("rename this function to `main`, or remove the attribute"));
    }
    if let Some(a) = &sig.asyncness {
        return err(
            a.span(),
            "`#[roost::main]` does not support `async fn main`",
        );
    }
    if let Some(c) = &sig.constness {
        return err(c.span(), "`fn main` cannot be `const`");
    }
    if let Some(u) = &sig.unsafety {
        return err(u.span(), "`fn main` cannot be `unsafe`");
    }
    if !sig.generics.params.is_empty() || sig.generics.where_clause.is_some() {
        return err(sig.generics.span(), "`fn main` cannot be generic");
    }
    if !sig.inputs.is_empty() {
        return Err(
            syn::Error::new(sig.inputs.span(), "`fn main` takes no parameters").help(
                "Java's `String[] args` is `std::env::args()` in Rust: \
             `let args: Vec<String> = std::env::args().skip(1).collect();`",
            ),
        );
    }
    Ok(())
}

pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_args(attr);
    let func = syn::parse2::<ItemFn>(item.clone());
    let (args, func) = match (args, func) {
        (Ok(a), Ok(f)) => (a, f),
        (Err(e), Ok(_)) => return with_item(e, item),
        (_, Err(e)) => {
            let e = syn::Error::new(e.span(), "`#[roost::main]` goes on `fn main`");
            return with_item(e, item);
        }
    };
    if let Err(e) = check_signature(&func) {
        if func.sig.ident == "main" {
            // Only the error: the item as written would add rustc's own complaints about
            // `main`'s signature.
            let e = crate::diag::emit([&e]);
            return quote!(#e fn main() {});
        }
        return with_item(e, item);
    }

    let ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = func;
    let ret = &sig.output;
    let mut fields = TokenStream::new();
    if let Some(n) = args.stack {
        fields.extend(quote!(stack: #n,));
    }
    if let Some(n) = args.gc_threshold {
        fields.extend(quote!(gc_threshold: ::core::option::Option::Some(#n),));
    }
    if args.gc_stats {
        fields.extend(quote!(gc_stats: true,));
    }
    let outcome = if is_unit(ret) {
        quote!({
            __roost_main();
            0
        })
    } else {
        // Spanned at the return type: a type that isn't `Result<(), E: Debug>` is reported
        // there.
        let ReturnType::Type(_, ty) = ret else {
            unreachable!("not unit")
        };
        quote_spanned!(ty.span()=>
            {
            match <#ty as ::roost::runtime::MainResult>::into_result(__roost_main()) {
                ::core::result::Result::Ok(()) => 0,
                ::core::result::Result::Err(__roost_error) => {
                    #[allow(unused_imports)]
                    use ::roost::runtime::{
                        ReportBoxedError as _, ReportDebug as _, ReportDisplay as _,
                        ReportError as _,
                    };
                    (&&&&::roost::runtime::Report(&__roost_error)).roost_report();
                    1
                }
            }
            }
        )
    };
    quote! {
        #(#attrs)*
        #vis fn main() {
            fn __roost_main() #ret #block

            #[allow(clippy::needless_update)]
            let config = ::roost::runtime::Config {
                #fields
                ..::core::default::Default::default()
            };
            ::roost::runtime::run(config, || #outcome)
        }
    }
}

/// The error, followed by the item unchanged (so that it still gets checked and no
/// "`main` function not found" error piles on).
fn with_item(e: syn::Error, item: TokenStream) -> TokenStream {
    let e = crate::diag::emit([&e]);
    quote!(#e #item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size_in_bytes("64MB"), Ok(64 << 20));
        assert_eq!(size_in_bytes("64 MiB"), Ok(64 << 20));
        assert_eq!(size_in_bytes("512kb"), Ok(512 << 10));
        assert_eq!(size_in_bytes("1G"), Ok(1 << 30));
        assert_eq!(size_in_bytes("4096"), Ok(4096));
        assert!(size_in_bytes("64XB").unwrap_err().contains("invalid size"));
        assert!(size_in_bytes("MB").is_err());
        assert!(size_in_bytes("0MB").unwrap_err().contains("zero"));
        assert!(size_in_bytes("99999999999999999999GB").is_err());
    }

    fn error_of(attr: TokenStream, item: TokenStream) -> String {
        let out = expand(attr, item).to_string();
        let start = out.find("compile_error").expect("an error") + "compile_error".len();
        out[start..].split('"').nth(1).unwrap().to_string()
    }

    #[test]
    fn argument_errors() {
        assert!(
            error_of(
                quote!(stak = "1MB"),
                quote!(
                    fn main() {}
                )
            )
            .contains("unknown argument `stak`")
        );
        assert!(
            error_of(
                quote!(stack = "lots"),
                quote!(
                    fn main() {}
                )
            )
            .contains("invalid size")
        );
        assert!(
            error_of(
                quote!(gc_stats = true),
                quote!(
                    fn main() {}
                )
            )
            .contains("takes no value")
        );
        assert!(
            error_of(
                quote!(stack = "1MB", stack = "2MB"),
                quote!(
                    fn main() {}
                )
            )
            .contains("twice")
        );
    }

    #[test]
    fn signature_errors() {
        assert!(
            error_of(
                quote!(),
                quote!(
                    fn start() {}
                )
            )
            .contains("not on other functions")
        );
        assert!(
            error_of(
                quote!(),
                quote!(
                    async fn main() {}
                )
            )
            .contains("async")
        );
        assert!(
            error_of(
                quote!(),
                quote!(
                    fn main(x: i32) {}
                )
            )
            .contains("no parameters")
        );
        assert!(
            error_of(
                quote!(),
                quote!(
                    struct S;
                )
            )
            .contains("goes on `fn main`")
        );
    }

    #[test]
    fn valid_main_expands_without_errors() {
        let out = expand(
            quote!(stack = "8MB", gc_stats),
            quote!(
                fn main() {}
            ),
        )
        .to_string();
        assert!(!out.contains("compile_error"), "{out}");
        assert!(out.contains("8388608"), "{out}");
    }
}
