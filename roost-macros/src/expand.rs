//! Code generation. For `class Dog extends Animal` this emits:
//!
//! ```text
//! type Dog = dyn DogClass;                       // what users write: Ref<Dog>
//! trait DogClass: AnimalClass {                  // the vtable part Dog introduces
//!     fn __roost_field_Dog_tricks(&self) -> &GcCell<i32>;   // one accessor per field
//!     fn fetch(&self) -> String;                 // methods Dog introduces
//! }
//! struct __RoostDog {                            // an object whose class is exactly Dog:
//!     __Animal_name: GcCell<String>,             // every field of every class in the
//!     __Dog_tricks: GcCell<i32>,                 // chain, flat, root first
//! }
//! impl Trace / Finalize / Object for __RoostDog  // Object: class name, cast targets
//! impl AnimalClass for __RoostDog { .. }         // one impl per class in the chain; each
//! impl DogClass for __RoostDog { .. }            //   method forwards to the nearest impl
//! impl dyn DogClass {
//!     fn new(..) -> Ref<Dog>;                    // allocate with defaults, then construct
//!     fn __roost_construct<T: ?Sized + DogClass>(this: &T, ..);   // super(..) + body
//!     fn __roost_method_speak<T: ?Sized + DogClass>(this: &T);   // Dog's speak
//!     fn __roost_private_helper<T>(..);         // private methods (not virtual)
//!     fn some_static(..);                        // static methods
//! }
//! ```
//!
//! Bodies are generic over `T`, the concrete class of `this`: they are monomorphized per
//! concrete class, so `this.speak()` inside Animal's constructor, run for a `__RoostDog`,
//! calls Dog's `speak` (Java semantics), and `super.speak()` is a direct call.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::visit_mut::VisitMut;
use syn::{Attribute, Ident, Pat, PatType, ReturnType};

use crate::index::{self, Index};
use crate::model::{Access, Class, Method};
use crate::rewrite::{Rewriter, take_super_call};

pub fn expand(index: &Index, errors: &mut Vec<syn::Error>) -> TokenStream {
    let mut out = TokenStream::new();
    for ci in 0..index.classes.len() {
        out.extend(expand_class(index, ci, errors));
    }
    out
}

/// A parameter's name for a signature without a body: the binding if it's a plain
/// identifier, else a placeholder.
fn param_name(p: &PatType, i: usize) -> Ident {
    match &*p.pat {
        Pat::Ident(pi) if pi.by_ref.is_none() && pi.subpat.is_none() => pi.ident.clone(),
        _ => format_ident!("__roost_arg{}", i),
    }
}

/// `name: Type` for each parameter (no patterns: for declarations and forwarding).
fn plain_params(params: &[PatType]) -> (Vec<Ident>, Vec<TokenStream>) {
    let names: Vec<Ident> = params
        .iter()
        .enumerate()
        .map(|(i, p)| param_name(p, i))
        .collect();
    let decls = params
        .iter()
        .zip(&names)
        .map(|(p, n)| {
            let ty = &p.ty;
            quote!(#n: #ty)
        })
        .collect();
    (names, decls)
}

fn is_doc(a: &Attribute) -> bool {
    a.path().is_ident("doc")
}

fn rewrite_body(
    index: &Index,
    ci: usize,
    body: &syn::Block,
    errors: &mut Vec<syn::Error>,
) -> syn::Block {
    let mut body = body.clone();
    Rewriter {
        index,
        class: ci,
        errors,
    }
    .visit_block_mut(&mut body);
    body
}

fn expand_class(index: &Index, ci: usize, errors: &mut Vec<syn::Error>) -> TokenStream {
    let class = &index.classes[ci];
    let chain: Vec<&Class> = index.chains[ci]
        .iter()
        .map(|&c| &index.classes[c])
        .collect();
    let vis = class.access.to_rust();
    let name = &class.name;
    let name_str = name.to_string();
    let tr = index::trait_name(class);
    let st = index::struct_name(class);
    let class_attrs = &class.attrs;

    let supertrait = match index.parent(ci) {
        Some(p) => {
            let p = index::trait_name(&index.classes[p]);
            quote!(#p)
        }
        None => quote!(::roost::Object),
    };

    // --- the trait: accessors for this class's fields, methods this class introduces.
    let mut trait_items = TokenStream::new();
    for f in &class.fields {
        let acc = index::accessor_name(class, &f.name);
        let ty = &f.ty;
        trait_items.extend(quote! {
            #[doc(hidden)]
            #[allow(non_snake_case, dead_code)]
            fn #acc(&self) -> &::roost::gc::GcCell<#ty>;
        });
    }
    for m in class.methods.iter().filter(|m| index.introduces(ci, m)) {
        let mname = m.name();
        let docs = m.attrs.iter().filter(|a| is_doc(a));
        let params: Vec<PatType> = m.params().cloned().collect();
        let (_, decls) = plain_params(&params);
        let ret = &m.sig.output;
        trait_items.extend(quote! {
            #(#docs)*
            fn #mname(&self #(, #decls)*) #ret;
        });
    }
    let trait_doc = format!(
        "The class `{name_str}`'s methods. Use the type `{name_str}` (`Ref<{name_str}>`); \
         this trait is what makes it polymorphic."
    );

    // --- the struct: every field of every class in the chain, root first.
    let mut storage = Vec::new(); // (storage ident, type, initializer, docs)
    for c in chain.iter().rev() {
        for f in &c.fields {
            let init = match &f.init {
                Some(e) => quote_spanned!(e.span()=> #e),
                None => {
                    let ty = &f.ty;
                    quote_spanned!(ty.span()=> <#ty as ::core::default::Default>::default())
                }
            };
            let docs: Vec<&Attribute> = f.attrs.iter().filter(|a| is_doc(a)).collect();
            storage.push((index::storage_name(c, &f.name), &f.ty, init, docs));
        }
    }
    let storage_decls = storage
        .iter()
        .map(|(s, ty, _, docs)| quote!(#(#docs)* #s: ::roost::gc::GcCell<#ty>));
    let storage_inits = storage.iter().map(|(s, ty, init, _)| {
        quote!(#s: ::roost::gc::GcCell::new({ let __roost_init: #ty = #init; __roost_init }))
    });
    let trace_asserts = storage.iter().map(
        |(_, ty, _, _)| quote_spanned!(ty.span()=> ::roost::__private::assert_trace::<#ty>();),
    );
    let trace = |method: TokenStream| -> TokenStream {
        let calls = storage.iter().map(
            |(s, ty, _, _)| quote_spanned!(ty.span()=> ::roost::gc::Trace::#method(&self.#s);),
        );
        quote!(#(#calls)*)
    };
    let (t_trace, t_root, t_unroot, t_glue) = (
        trace(quote!(trace)),
        trace(quote!(root)),
        trace(quote!(unroot)),
        trace(quote!(finalize_glue)),
    );
    let unsafe_block = |body: TokenStream| {
        if storage.is_empty() {
            quote!()
        } else {
            quote!(unsafe { #body })
        }
    };
    let (u_trace, u_root, u_unroot) = (
        unsafe_block(t_trace),
        unsafe_block(t_root),
        unsafe_block(t_unroot),
    );

    let cast_targets = chain.iter().map(|c| {
        let t = index::trait_name(c);
        quote!(.offer::<dyn #t, _>(self))
    });

    // --- one trait impl per class in the chain.
    let mut trait_impls = TokenStream::new();
    for (&ai, a) in index.chains[ci].iter().zip(&chain) {
        let atr = index::trait_name(a);
        let mut items = TokenStream::new();
        for f in &a.fields {
            let acc = index::accessor_name(a, &f.name);
            let s = index::storage_name(a, &f.name);
            let ty = &f.ty;
            items.extend(quote! {
                fn #acc(&self) -> &::roost::gc::GcCell<#ty> { &self.#s }
            });
        }
        for m in a.methods.iter().filter(|m| index.introduces(ai, m)) {
            let mname = m.name();
            let owner = index
                .resolve_virtual(ci, mname)
                .expect("an introduced method resolves");
            let owner_class = &index.classes[owner];
            let impl_m = owner_class
                .methods
                .iter()
                .find(|om| om.is_virtual() && om.name() == mname)
                .expect("resolve_virtual found it");
            let otr = index::trait_name(owner_class);
            let f = index::method_impl_name(mname);
            // The overriding method's own signature: if it doesn't match the declaration,
            // rustc's E0053 points at both.
            let params: Vec<PatType> = impl_m.params().cloned().collect();
            let (names, decls) = plain_params(&params);
            let ret = &impl_m.sig.output;
            items.extend(quote! {
                #[inline]
                fn #mname(&self #(, #decls)*) #ret {
                    <dyn #otr>::#f(self #(, #names)*)
                }
            });
        }
        trait_impls.extend(quote! {
            impl #atr for #st { #items }
        });
    }

    // --- inherent impl on `dyn DogClass`: constructor, method bodies, statics.
    let mut inherent = TokenStream::new();
    inherent.extend(constructor(index, ci, errors));
    for m in &class.methods {
        inherent.extend(method(index, ci, m, errors));
    }

    quote! {
        #(#class_attrs)*
        #vis type #name = dyn #tr;

        #[doc = #trait_doc]
        #vis trait #tr: #supertrait {
            #trait_items
        }

        #[doc(hidden)]
        #[allow(non_snake_case, non_camel_case_types, dead_code)]
        struct #st {
            #(#storage_decls,)*
        }

        impl #st {
            // A field type that isn't `Trace` is reported here, at the field's type.
            #[allow(dead_code)]
            fn __roost_assert_fields() {
                #(#trace_asserts)*
            }

            #[allow(clippy::default_trait_access)]
            fn __roost_default() -> Self {
                #st { #(#storage_inits,)* }
            }
        }

        impl ::roost::gc::Finalize for #st {}

        // SAFETY: forwards to every field; the struct holds nothing else.
        unsafe impl ::roost::gc::Trace for #st {
            unsafe fn trace(&self) { #u_trace }
            unsafe fn root(&self) { #u_root }
            unsafe fn unroot(&self) { #u_unroot }
            fn finalize_glue(&self) {
                ::roost::gc::Finalize::finalize(self);
                #t_glue
            }
        }

        impl ::roost::Object for #st {
            fn __roost_casts(&self, req: &mut ::roost::CastRequest<'_>) {
                req.offer_class(self)
                    #(#cast_targets)*
                    .offer::<dyn ::roost::Object, _>(self);
            }
            fn class_name(&self) -> &'static str {
                #name_str
            }
        }

        #trait_impls

        impl dyn #tr {
            #inherent
        }
    }
}

fn constructor(index: &Index, ci: usize, errors: &mut Vec<syn::Error>) -> TokenStream {
    let class = &index.classes[ci];
    let tr = index::trait_name(class);
    let st = index::struct_name(class);
    let construct = Ident::new(index::CONSTRUCT, Span::call_site());

    let (attrs, access, params, mut body, span) = match &class.ctor {
        Some(c) => (
            c.attrs.clone(),
            c.access,
            c.params.clone(),
            c.body.clone(),
            c.name.span(),
        ),
        None => (
            Vec::new(),
            class.access,
            Vec::new(),
            syn::parse_quote!({}),
            class.name.span(),
        ),
    };
    let vis = access.to_rust();
    let super_call = take_super_call(&mut body);
    let body = rewrite_body(index, ci, &body, errors);

    let super_stmt = match (index.parent(ci), super_call) {
        (Some(p), Some((sspan, mut args))) => {
            let ptr = index::trait_name(&index.classes[p]);
            let mut rw = Rewriter {
                index,
                class: ci,
                errors,
            };
            for a in args.iter_mut() {
                rw.visit_expr_mut(a);
            }
            quote_spanned!(sspan=> <dyn #ptr>::#construct(this, #args);)
        }
        (Some(p), None) => {
            let parent = &index.classes[p];
            if parent.ctor.as_ref().is_some_and(|c| !c.params.is_empty()) {
                errors.push(syn::Error::new(
                    span,
                    format!(
                        "`{}` has no no-arg constructor: call `super(...)` as the first \
                         statement of a constructor",
                        parent.name
                    ),
                ));
            }
            let ptr = index::trait_name(parent);
            quote!(<dyn #ptr>::#construct(this);)
        }
        (None, Some((sspan, args))) => {
            if !args.is_empty() {
                errors.push(syn::Error::new(
                    sspan,
                    "a class without `extends` has no superclass constructor to pass arguments to",
                ));
            }
            quote!()
        }
        (None, None) => quote!(),
    };

    let (names, decls) = plain_params(&params);
    let stmts = &body.stmts;
    let doc = format!(
        "`new {}(..)`: allocates the object, then runs the constructors.",
        class.name
    );
    quote! {
        #[doc(hidden)]
        #[allow(clippy::used_underscore_binding)]
        fn #construct<T: ?Sized + #tr>(this: &T #(, #params)*) {
            let _ = this;
            #super_stmt
            #(#stmts)*
        }

        #[doc = #doc]
        #(#attrs)*
        #[allow(clippy::new_ret_no_self)]
        #vis fn new(#(#decls),*) -> ::roost::Ref<dyn #tr> {
            // Java order: every field gets its default first, then the constructors run
            // root-first, with `this` already the complete (most-derived) object.
            let object = ::roost::Ref::new(#st::__roost_default());
            <dyn #tr>::#construct(&*object #(, #names)*);
            object
        }
    }
}

fn method(index: &Index, ci: usize, m: &Method, errors: &mut Vec<syn::Error>) -> TokenStream {
    let class = &index.classes[ci];
    let tr = index::trait_name(class);
    let params = m.params();
    let ret: &ReturnType = &m.sig.output;
    let attrs = m.attrs.iter().filter(|a| !is_doc(a) || m.is_static);
    if m.is_static {
        let vis = m.access.to_rust();
        let (mname, body) = (m.name(), &m.body);
        return quote! {
            #(#attrs)*
            #vis fn #mname(#(#params),*) #ret #body
        };
    }
    let f = if m.access == Access::Private {
        index::private_impl_name(m.name())
    } else {
        index::method_impl_name(m.name())
    };
    let body = rewrite_body(index, ci, &m.body, errors);
    let stmts = &body.stmts;
    quote! {
        #[doc(hidden)]
        #(#attrs)*
        fn #f<T: ?Sized + #tr>(this: &T #(, #params)*) #ret {
            let _ = this;
            #(#stmts)*
        }
    }
}
