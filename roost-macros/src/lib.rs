//! Proc macros for roost. Use them through the `roost` crate (`roost::class!`), not directly.
//!
//! `class!` takes Java-style class declarations; see the `roost` crate docs and the README
//! for the language. Layout of this crate:
//!
//! * [`model`]: the syntax tree and its parser;
//! * [`index`]: name resolution (superclass chains, `this.field`, `super.m()`) and checks;
//! * [`rewrite`]: rewriting `this.field` / `super` in bodies;
//! * [`expand`]: code generation.

use proc_macro2::TokenStream;
use quote::quote;

mod expand;
mod index;
mod model;
mod rewrite;

/// Declares classes. See the `roost` crate.
#[proc_macro]
pub fn class(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand_class(input.into()).into()
}

fn expand_class(input: TokenStream) -> TokenStream {
    let hierarchy: model::Hierarchy = match syn::parse2(input) {
        Ok(h) => h,
        Err(e) => return e.to_compile_error(),
    };
    let mut errors = Vec::new();
    let code = match index::Index::build(hierarchy, &mut errors) {
        Some(index) => expand::expand(&index, &mut errors),
        None => TokenStream::new(),
    };
    let errors = errors.iter().map(syn::Error::to_compile_error);
    quote! {
        #(#errors)*
        #code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `compile_error!` messages `class!` produces for `input`.
    fn errors(input: TokenStream) -> Vec<String> {
        let out = expand_class(input).to_string();
        out.split("compile_error")
            .skip(1)
            .map(|s| {
                let start = s.find('"').unwrap() + 1;
                let end = start
                    + s[start..]
                        .find("\" }")
                        .or_else(|| s[start..].find('"'))
                        .unwrap();
                s[start..end].to_string()
            })
            .collect()
    }

    fn assert_error(input: TokenStream, expected: &str) {
        let errs = errors(input);
        assert!(
            errs.iter().any(|e| e.contains(expected)),
            "expected an error containing {expected:?}, got {errs:?}"
        );
    }

    #[test]
    fn valid_hierarchy_has_no_errors() {
        let errs = errors(quote! {
            public class A { protected x: i32; public A() {} public fn f() -> i32 { this.x } }
            public class B extends A { @Override public fn f() -> i32 { super.f() + this.x } }
            public class C extends B { public C() { super(); this.x += 1; } }
        });
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn override_is_checked_both_ways() {
        assert_error(
            quote! {
                class A { public fn f() {} }
                class B extends A { @Override public fn g() {} }
            },
            "method `g` does not override or implement a method",
        );
        assert_error(
            quote! {
                class A { public fn f() {} }
                class B extends A {}
                class C extends B { public fn f() {} }
            },
            "method `f` overrides `A.f` but is not marked @Override",
        );
    }

    #[test]
    fn private_members_are_not_inherited() {
        assert_error(
            quote! {
                class A { private x: i32; }
                class B extends A { public fn f() -> i32 { this.x } }
            },
            "field `x` is private to class `A`",
        );
        // A private method is not virtual: a same-named public method in a subclass is a
        // new method, not an override.
        let errs = errors(quote! {
            class A { private fn f() {} }
            class B extends A { public fn f() {} }
        });
        assert!(errs.is_empty(), "{errs:?}");
        assert_error(
            quote! { class A { public fn f() -> i32 { this.nope } } },
            "class `A` has no field `nope`",
        );
    }

    #[test]
    fn super_constructor_call_must_come_first() {
        assert_error(
            quote! {
                class A { A() {} }
                class B extends A { B() { let x = 1; super(); } }
            },
            "`super(...)` must be the first statement of a constructor",
        );
        assert_error(
            quote! {
                class A { A(x: i32) {} }
                class B extends A { B() {} }
            },
            "`A` has no no-arg constructor",
        );
        assert_error(
            quote! { class A { public fn f() { super.f(); } } },
            "`super` method call in a class without a superclass",
        );
        assert_error(
            quote! {
                class A {}
                class B extends A { public fn f() { super.g(); } }
            },
            "no method `g` in any superclass",
        );
    }

    #[test]
    fn partial_field_updates_are_rejected() {
        assert_error(
            quote! { class A { p: (i32, i32); public fn f() { this.p.0 = 1; } } },
            "cannot assign into a part of field `p`",
        );
        assert_error(
            quote! { class A { v: Vec<i32>; public fn f() { this.v[0] += 1; } } },
            "cannot update a part of field `v` in place",
        );
    }

    #[test]
    fn structural_errors() {
        assert_error(
            quote! { class A extends Nowhere {} },
            "unknown superclass `Nowhere`",
        );
        assert_error(
            quote! { class A extends B {} class B extends A {} },
            "cyclic inheritance",
        );
        assert_error(
            quote! { class A { public fn f(&self) {} } },
            "methods take no `self` parameter",
        );
        assert_error(
            quote! { class A { public fn class_name() -> String { String::new() } } },
            "`class_name` is reserved",
        );
        assert_error(
            quote! { class A { public fn to_string() -> String { String::new() } } },
            "overrides `Object.to_string` but is not marked @Override",
        );
    }

    #[test]
    fn interfaces_abstract_final_static() {
        let errs = errors(quote! {
            interface Named { fn name() -> String; }
            interface Pet extends Named { default fn greet() -> String { this.name() } }
            abstract class A implements Pet { static n: i32; public abstract fn f(); }
            final class B extends A {
                @Override public fn name() -> String { String::new() }
                @Override public fn f() { A::n += 1; Self::n = 0; }
            }
        });
        assert!(errs.is_empty(), "{errs:?}");
        assert_error(
            quote! {
                interface Named { fn name() -> String; }
                interface Pet extends Named { fn name() -> String; }
            },
            "`name` is already declared in super-interface `Named`",
        );
        assert_error(
            quote! { interface A extends B {} interface B extends A {} },
            "cyclic inheritance",
        );
        assert_error(
            quote! { class A { private static n: i32; } class B extends A { public fn f() -> i32 { A::n } } },
            "static field `n` is private to class `A`",
        );
        assert_error(
            quote! { class A { static n: i32; public fn f() -> i32 { this.n } } },
            "`n` is a static field of `A`",
        );
        assert_error(
            quote! {
                abstract class A { public fn f() {} }
                abstract class B extends A { @Override public abstract fn f(); }
            },
            "abstract method `f` cannot re-declare `A.f`",
        );
        assert_error(
            quote! {
                abstract class A { public abstract fn f(); }
                class B extends A { @Override public fn f() { super.f(); } }
            },
            "`A.f` is abstract: it cannot be called with `super`",
        );
        assert_error(
            quote! { interface I { fn to_string() -> String; } },
            "`to_string` is reserved",
        );
        let out = expand_class(quote! { class Box<T> {} }).to_string();
        assert!(out.contains("generic classes are not supported"), "{out}");
        let out = expand_class(quote! { abstract final class A {} }).to_string();
        assert!(out.contains("both `abstract` and `final`"), "{out}");
    }

    #[test]
    fn parse_errors_point_at_the_problem() {
        let out = expand_class(quote! { pub class A {} }).to_string();
        assert!(out.contains("write `public`"), "{out}");
        let out = expand_class(quote! { class A { @Deprecated fn f() {} } }).to_string();
        assert!(out.contains("only `@Override` is supported"), "{out}");
    }
}
