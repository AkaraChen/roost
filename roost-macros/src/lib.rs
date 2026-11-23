//! Proc macros for roost. Use them through the `roost` crate (`roost::class!`), not directly.
//!
//! Status: placeholder. The class DSL is designed in a later step (task #347); for now
//! `class!` only exists so the crate layout, the re-export from `roost` and the build
//! pipeline are in place. Any use is a compile error pointing at the invocation.

use proc_macro2::{Span, TokenStream};
use quote::quote_spanned;

/// Declares a class. **Not implemented yet**: every invocation is a compile error.
#[proc_macro]
pub fn class(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand_class(input.into()).into()
}

/// The message every `class!` invocation currently fails with.
const NOT_YET: &str = "roost::class! is a placeholder: the class DSL is not implemented yet";

fn expand_class(input: TokenStream) -> TokenStream {
    // Point the error at the first token the user wrote, so it lands on their code rather
    // than on the macro definition.
    let span = input
        .into_iter()
        .next()
        .map_or_else(Span::call_site, |token| token.span());
    quote_spanned! {span=> ::core::compile_error!(#NOT_YET); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn placeholder_expands_to_compile_error() {
        for input in [quote!(), quote!(class Animal {})] {
            let out = expand_class(input).to_string();
            assert!(out.contains("compile_error"), "{out}");
            assert!(out.contains("placeholder"), "{out}");
        }
    }
}
