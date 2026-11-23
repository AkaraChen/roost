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

use crate::index::{self, Impl, Index};
use crate::model::{Access, Class, Method};
use crate::rewrite::{Rewriter, take_super_call};

pub fn expand(index: &Index, errors: &mut Vec<syn::Error>) -> TokenStream {
    let mut out = shared_traits(index);
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

/// `String`, `std::string::String` or `alloc::string::String` (optionally with a leading
/// `::`), by spelling: the macro can't see through aliases or a shadowing `String`.
fn is_string(ty: &syn::Type) -> bool {
    let syn::Type::Path(tp) = ty else {
        return false;
    };
    let path = &tp.path;
    if tp.qself.is_some() || path.segments.iter().any(|s| !s.arguments.is_none()) {
        return false;
    }
    let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    match segs
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["String"] => path.leading_colon.is_none(),
        ["std" | "alloc", "string", "String"] => true,
        _ => false,
    }
}

fn is_doc(a: &Attribute) -> bool {
    a.path().is_ident("doc")
}

fn rewrite_body(
    index: &Index,
    ci: usize,
    has_this: bool,
    body: &syn::Block,
    errors: &mut Vec<syn::Error>,
) -> (syn::Block, TokenStream) {
    let mut body = body.clone();
    let before = errors.len();
    Rewriter {
        index,
        class: ci,
        has_this,
        errors,
    }
    .visit_block_mut(&mut body);
    if errors.len() > before {
        // The error is reported; drop the body so that rustc doesn't pile follow-up errors
        // on the placeholder expressions.
        return (
            syn::parse_quote!({ ::core::unreachable!() }),
            quote!(#[allow(unused_variables, unreachable_code)]),
        );
    }
    (body, TokenStream::new())
}

/// `fn name(&self, ..) -> R { <dyn OwnerTrait>::__roost_method_name(self, ..) }`: the vtable
/// entry of concrete class `ci` for method `name`, declared as `decl` somewhere up.
fn forward(index: &Index, ci: usize, decl: &Method) -> TokenStream {
    let mname = decl.name();
    let (sig_m, target) = match index.resolve_impl(ci, mname) {
        Some(Impl::Class(k) | Impl::Default(k)) => {
            let m = index
                .virtual_method(k, mname)
                .expect("resolve_impl found it");
            let tr = index::trait_name(&index.classes[k]);
            let f = index::method_impl_name(mname);
            (m, Some(quote!(<dyn #tr>::#f)))
        }
        // Missing implementation: reported by the index at the class. Keep the impl
        // complete so that is the only error.
        Some(Impl::Object) | None => (decl, None),
    };
    // The implementing method's own signature: if it doesn't match the declaration,
    // rustc's E0053 points at both.
    let params: Vec<PatType> = sig_m.params().cloned().collect();
    let (names, decls) = plain_params(&params);
    let ret = &sig_m.sig.output;
    let body = match target {
        Some(t) => quote!(#t(self #(, #names)*)),
        None => quote!({ #(let _ = #names;)* ::core::unreachable!() }),
    };
    quote! {
        #[inline]
        fn #mname(&self #(, #decls)*) #ret { #body }
    }
}

/// Supertraits of `t`'s trait: the parent class trait (or `ObjectClass`) and the traits of
/// the interfaces it lists.
fn supertraits(index: &Index, t: usize) -> TokenStream {
    let class = &index.classes[t];
    let mut supers = vec![match index.parent(t) {
        Some(p) => {
            let p = index::trait_name(&index.classes[p]);
            quote!(#p)
        }
        None => quote!(::roost::ObjectClass),
    }];
    for i in &class.interfaces {
        let i = index::trait_name(&index.classes[index.find(i).expect("resolved")]);
        supers.push(quote!(#i));
    }
    for g in index.shared.iter().filter(|g| g.decls.contains(&t)) {
        let s = index::shared_trait_name(index, g);
        supers.push(quote!(#s));
    }
    quote!(#(#supers)+*)
}

/// The trait's own items: field accessors, and the methods `t` introduces.
fn trait_items(index: &Index, t: usize) -> TokenStream {
    let class = &index.classes[t];
    let mut items = TokenStream::new();
    for f in class.instance_fields() {
        let acc = index::accessor_name(class, &f.name);
        let ty = &f.ty;
        items.extend(quote! {
            #[doc(hidden)]
            #[allow(non_snake_case, dead_code)]
            fn #acc(&self) -> &::roost::gc::GcCell<#ty>;
        });
    }
    for m in class.methods.iter().filter(|m| index.introduces(t, m)) {
        if index.shared_of(t, m.name()).is_none() {
            items.extend(method_decl(m));
        }
    }
    items
}

/// `fn m(&self, ..) -> R;`, with the method's docs.
fn method_decl(m: &Method) -> TokenStream {
    let mname = m.name();
    let docs = m.attrs.iter().filter(|a| is_doc(a));
    let params: Vec<PatType> = m.params().cloned().collect();
    let (_, decls) = plain_params(&params);
    let ret = &m.sig.output;
    quote! {
        #(#docs)*
        fn #mname(&self #(, #decls)*) #ret;
    }
}

/// The traits of [`index::Shared`] methods.
fn shared_traits(index: &Index) -> TokenStream {
    let mut out = TokenStream::new();
    for g in &index.shared {
        let name = index::shared_trait_name(index, g);
        let decl = index.virtual_method(g.decls[0], &g.name).expect("declared");
        let item = method_decl(decl);
        let doc = format!(
            "`{}`, declared by {}: one trait, so that it isn't ambiguous where they meet.",
            g.name,
            g.decls
                .iter()
                .map(|&d| format!("`{}`", index.classes[d].name))
                .collect::<Vec<_>>()
                .join(", ")
        );
        out.extend(quote! {
            #[doc = #doc]
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            pub trait #name: ::roost::ObjectClass {
                #item
            }
        });
    }
    out
}

fn expand_interface(index: &Index, ii: usize, errors: &mut Vec<syn::Error>) -> TokenStream {
    let iface = &index.classes[ii];
    let vis = iface.access.to_rust();
    let name = &iface.name;
    let tr = index::trait_name(iface);
    let attrs = &iface.attrs;
    let supers = supertraits(index, ii);
    let items = trait_items(index, ii);
    let doc = format!(
        "The interface `{name}`'s methods. Use the type `{name}` (`Ref<{name}>`); this trait is \
         what makes it polymorphic."
    );
    let mut inherent = TokenStream::new();
    for m in iface.methods.iter().filter(|m| !m.is_abstract()) {
        inherent.extend(method(index, ii, m, errors));
    }
    quote! {
        #(#attrs)*
        #vis type #name = dyn #tr;

        #[doc = #doc]
        #vis trait #tr: #supers {
            #items
        }

        impl dyn #tr {
            #inherent
        }
    }
}

fn expand_class(index: &Index, ci: usize, errors: &mut Vec<syn::Error>) -> TokenStream {
    let class = &index.classes[ci];
    if class.is_interface() {
        return expand_interface(index, ci, errors);
    }
    let chain: Vec<&Class> = index.chains[ci]
        .iter()
        .map(|&c| &index.classes[c])
        .collect();
    let vis = class.access.to_rust();
    let name = &class.name;
    let name_str = name.to_string();
    let tr = index::trait_name(class);
    let class_attrs = &class.attrs;
    let supers = supertraits(index, ci);
    let items = trait_items(index, ci);
    let trait_doc = format!(
        "The class `{name_str}`'s methods. Use the type `{name_str}` (`Ref<{name_str}>`); \
         this trait is what makes it polymorphic."
    );

    // --- static fields: thread-locals, like the collector.
    let statics = class.static_fields().map(|f| {
        let key = index::static_name(class, &f.name);
        let ty = &f.ty;
        let docs = f.attrs.iter().filter(|a| is_doc(a));
        let init = match &f.init {
            Some(e) => quote_spanned!(e.span()=> #e),
            None => quote_spanned!(ty.span()=> <#ty as ::core::default::Default>::default()),
        };
        quote! {
            ::std::thread_local! {
                #(#docs)*
                #[allow(non_upper_case_globals, clippy::default_trait_access)]
                static #key: ::roost::gc::GcCell<#ty> =
                    ::roost::gc::GcCell::new({ let __roost_init: #ty = #init; __roost_init });
            }
        }
    });

    // --- inherent impl on `dyn DogClass`: constructor, method bodies, statics.
    let mut inherent = TokenStream::new();
    inherent.extend(constructor(index, ci, errors));
    for m in class.methods.iter().filter(|m| !m.is_abstract()) {
        inherent.extend(method(index, ci, m, errors));
    }

    let concrete = if class.abstract_.is_some() {
        abstract_marker(class)
    } else {
        concrete_class(index, ci, &chain)
    };

    quote! {
        #(#class_attrs)*
        #vis type #name = dyn #tr;

        #[doc = #trait_doc]
        #vis trait #tr: #supers {
            #items
        }

        #(#statics)*

        #concrete

        impl dyn #tr {
            #inherent
        }
    }
}

/// `Shape::new(..)` on an abstract class: an error at the call, with this message.
fn abstract_marker(class: &Class) -> TokenStream {
    let marker = index::abstract_marker_name(class);
    let msg = format!("cannot instantiate abstract class `{}`", class.name);
    let label = format!("`{}` is abstract", class.name);
    quote! {
        #[doc(hidden)]
        #[diagnostic::on_unimplemented(
            message = #msg,
            label = #label,
            note = "create an instance of a concrete subclass instead"
        )]
        #[allow(non_camel_case_types)]
        trait #marker {}
    }
}

/// The struct for objects whose class is exactly `ci`, and its trait impls.
fn concrete_class(index: &Index, ci: usize, chain: &[&Class]) -> TokenStream {
    let class = &index.classes[ci];
    let name_str = class.name.to_string();
    let st = index::struct_name(class);

    // --- the struct: every field of every class in the chain, root first.
    let mut storage = Vec::new(); // (storage ident, type, initializer, docs)
    for c in chain.iter().rev() {
        for f in c.instance_fields() {
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

    let cast_targets = index.supertypes(ci).map(|t| {
        let t = index::trait_name(&index.classes[t]);
        quote!(.offer::<dyn #t, _>(self))
    });

    // `toString` / `equals` / `hashCode` overrides.
    let object_overrides = index::OBJECT_METHODS.iter().filter_map(|m| {
        let m = Ident::new(m, Span::call_site());
        match index.resolve_impl(ci, &m) {
            Some(Impl::Class(k)) => Some(forward(
                index,
                ci,
                index.virtual_method(k, &m).expect("resolved"),
            )),
            _ => None,
        }
    });

    // --- one trait impl per class in the chain and per interface.
    let mut trait_impls = TokenStream::new();
    for t in index.supertypes(ci) {
        let a = &index.classes[t];
        let atr = index::trait_name(a);
        let mut items = TokenStream::new();
        for f in a.instance_fields() {
            let acc = index::accessor_name(a, &f.name);
            let s = index::storage_name(a, &f.name);
            let ty = &f.ty;
            items.extend(quote! {
                fn #acc(&self) -> &::roost::gc::GcCell<#ty> { &self.#s }
            });
        }
        for m in a.methods.iter().filter(|m| index.introduces(t, m)) {
            if index.shared_of(t, m.name()).is_none() {
                items.extend(forward(index, ci, m));
            }
        }
        trait_impls.extend(quote! {
            impl #atr for #st { #items }
        });
    }
    let supertypes: Vec<usize> = index.supertypes(ci).collect();
    for g in index
        .shared
        .iter()
        .filter(|g| g.decls.iter().any(|d| supertypes.contains(d)))
    {
        let s = index::shared_trait_name(index, g);
        let decl = index.virtual_method(g.decls[0], &g.name).expect("declared");
        let item = forward(index, ci, decl);
        trait_impls.extend(quote! {
            impl #s for #st { #item }
        });
    }

    let tr = index::trait_name(class);
    quote! {
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

            fn __roost_new() -> ::roost::Ref<dyn #tr> {
                ::roost::Ref::new(#st::__roost_default())
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

        impl ::roost::ObjectClass for #st {
            fn __roost_casts(&self, req: &mut ::roost::CastRequest<'_>) {
                req.offer_class(self)
                    #(#cast_targets)*
                    .offer::<dyn ::roost::ObjectClass, _>(self);
            }
            fn class_name(&self) -> &'static str {
                #name_str
            }
            #(#object_overrides)*
        }

        #trait_impls
    }
}

fn constructor(index: &Index, ci: usize, errors: &mut Vec<syn::Error>) -> TokenStream {
    let class = &index.classes[ci];
    let tr = index::trait_name(class);
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
    let (body, ctor_allow) = rewrite_body(index, ci, true, &body, errors);

    let super_stmt = match (index.parent(ci), super_call) {
        (Some(p), Some((sspan, mut args))) => {
            let ptr = index::trait_name(&index.classes[p]);
            let mut rw = Rewriter {
                index,
                class: ci,
                has_this: true,
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

    // `String` parameters are taken as `impl Into<String>` (by `new` and by the constructor
    // body, so `super("Rex")` works too) and converted on entry, so `Dog::new("Rex")` works.
    // Only `String`: see "构造：`new!` 和构造器参数" in the README for why not every type.
    let (names, decls) = plain_params(&params);
    let mut ctor_params = Vec::new();
    let mut conversions = Vec::new();
    for (p, n) in params.iter().zip(&names) {
        let ty = &p.ty;
        if is_string(ty) {
            let pat = &p.pat;
            ctor_params.push(quote_spanned!(ty.span()=> #n: impl ::core::convert::Into<#ty>));
            conversions.push(quote!(let #pat: #ty = ::core::convert::Into::<#ty>::into(#n);));
        } else {
            ctor_params.push(quote!(#p));
        }
    }
    // `new` takes the same parameters, without patterns.
    let new_decls = params.iter().zip(&names).zip(&decls).map(|((p, n), decl)| {
        let ty = &p.ty;
        if is_string(ty) {
            quote_spanned!(ty.span()=> #n: impl ::core::convert::Into<#ty>)
        } else {
            decl.clone()
        }
    });
    let new_decls: Vec<TokenStream> = new_decls.collect();
    let stmts = &body.stmts;
    let new = if class.abstract_.is_some() {
        // Never callable: the bound fails at the call site with the marker's message. (The
        // lifetime keeps the bound from being "trivially false" at the definition.)
        let marker = index::abstract_marker_name(class);
        quote! {
            #[doc(hidden)]
            #[allow(private_bounds, unused_variables, clippy::new_ret_no_self)]
            #vis fn new<'__roost>(#(#new_decls),*) -> ::roost::Ref<dyn #tr>
            where
                &'__roost (): #marker,
            {
                ::core::unreachable!()
            }
        }
    } else {
        let st = index::struct_name(class);
        let doc = format!(
            "`new {}(..)`: allocates the object, then runs the constructors.",
            class.name
        );
        quote! {
            #[doc = #doc]
            #(#attrs)*
            #[allow(clippy::new_ret_no_self)]
            #vis fn new(#(#new_decls),*) -> ::roost::Ref<dyn #tr> {
                // Java order: every field gets its default first, then the constructors run
                // root-first, with `this` already the complete (most-derived) object.
                let object = #st::__roost_new();
                <dyn #tr>::#construct(&*object #(, #names)*);
                object
            }
        }
    };
    quote! {
        #[doc(hidden)]
        #[allow(clippy::used_underscore_binding)]
        #ctor_allow
        fn #construct<T: ?Sized + #tr>(this: &T #(, #ctor_params)*) {
            let _ = this;
            #(#conversions)*
            #super_stmt
            #(#stmts)*
        }

        #new
    }
}

fn method(index: &Index, ci: usize, m: &Method, errors: &mut Vec<syn::Error>) -> TokenStream {
    let class = &index.classes[ci];
    let tr = index::trait_name(class);
    let params = m.params();
    let ret: &ReturnType = &m.sig.output;
    let attrs = m.attrs.iter().filter(|a| !is_doc(a) || m.is_static);
    let body = m
        .body
        .as_ref()
        .expect("abstract methods have no body to generate");
    if m.is_static {
        let vis = m.access.to_rust();
        let mname = m.name();
        let (body, allow) = rewrite_body(index, ci, false, body, errors);
        return quote! {
            #(#attrs)*
            #allow
            #vis fn #mname(#(#params),*) #ret #body
        };
    }
    let f = if m.access == Access::Private {
        index::private_impl_name(m.name())
    } else {
        index::method_impl_name(m.name())
    };
    let (body, allow) = rewrite_body(index, ci, true, body, errors);
    let stmts = &body.stmts;
    quote! {
        #[doc(hidden)]
        #(#attrs)*
        #allow
        fn #f<T: ?Sized + #tr>(this: &T #(, #params)*) #ret {
            let _ = this;
            #(#stmts)*
        }
    }
}
