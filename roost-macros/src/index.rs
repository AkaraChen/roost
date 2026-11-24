//! Name resolution over a parsed [`Hierarchy`]: superclass chains, implemented interfaces,
//! which class a `this.field` / `super.m()` refers to, which implementation each method of
//! each concrete class uses, and the semantic checks (`@Override`, abstract, final, ...).
//!
//! Classes and interfaces share one list (`Index::classes`); an interface is a `Class` with
//! `kind == Interface`, no fields and no superclass.

use std::collections::BTreeMap;

use proc_macro2::Span;
use quote::{ToTokens, format_ident};
use syn::Ident;

use crate::diag::{Help, similar};
use crate::model::{Access, Class, Hierarchy, Method};

/// Separates a message from its help line in the `Err(String)`s of the resolve functions;
/// see [`error`].
pub const HELP: &str = "\u{1}";

/// The error for a message from a resolve function: `msg` or `msg{HELP}help`.
pub fn error(span: Span, msg: &str) -> syn::Error {
    match msg.split_once(HELP) {
        Some((m, h)) => syn::Error::new(span, m).help(h),
        None => syn::Error::new(span, msg),
    }
}

/// Names that can't be methods: other `roost::ObjectClass` methods, and the constructor.
const RESERVED: &[&str] = &["class_name", "as_any", "new"];

/// `java.lang.Object` methods a class may override (with `@Override`).
pub const OBJECT_METHODS: &[&str] = &["to_string", "equals", "hash_code"];

pub struct Index {
    pub classes: Vec<Class>,
    /// `chains[i]`: class `i`, its parent, ..., the root (indices into `classes`). For an
    /// interface, just `[i]`.
    pub chains: Vec<Vec<usize>>,
    /// `ifaces[i]`: every interface class `i` implements, directly or through its
    /// superclasses or super-interfaces, without duplicates. For an interface: its
    /// super-interfaces (not itself).
    pub ifaces: Vec<Vec<usize>>,
    pub shared: Vec<Shared>,
}

/// A method declared by several related types: say class `Book` declares `title()` and
/// `Novel extends Book implements Titled`, where interface `Titled` declares it too. If
/// `BookClass` and `TitledInterface` both declared `title`, `novel.title()` would be
/// ambiguous (E0034) and no inherent method can fix that for a trait object. So the
/// method goes into one trait of its own, `__RoostBook_title`, which both have as a
/// supertrait instead: reached along two paths, it is still one method.
pub struct Shared {
    pub name: Ident,
    /// The declaring types (indices into `classes`), sorted.
    pub decls: Vec<usize>,
}

/// A field as seen from some class: who declares it.
pub struct FieldRef {
    pub owner: usize,
}

/// Where the method a class's vtable entry runs comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Impl {
    /// The body declared in this class.
    Class(usize),
    /// A `default` method of this interface.
    Default(usize),
    /// `roost::ObjectClass`'s default (`to_string`, `equals`, `hash_code`).
    Object,
}

/// Where a method an override refers to is declared.
enum Above {
    Type(usize),
    Object,
}

impl Index {
    pub fn build(h: Hierarchy, errors: &mut Vec<syn::Error>) -> Option<Index> {
        let classes = h.classes;
        let mut ok = true;
        for (i, c) in classes.iter().enumerate() {
            if let Some(prev) = classes[..i].iter().find(|p| p.name == c.name) {
                let mut e =
                    syn::Error::new(c.name.span(), format!("`{}` is declared twice", c.name));
                e.combine(syn::Error::new(prev.name.span(), "first declared here"));
                errors.push(e);
                ok = false;
            }
        }
        let names: Vec<String> = classes.iter().map(|c| c.name.to_string()).collect();
        let find = |name: &Ident, errors: &mut Vec<syn::Error>, what: &str| {
            let found = classes.iter().position(|c| c.name == *name);
            if found.is_none() {
                let e = syn::Error::new(name.span(), format!("unknown {what} `{name}`"));
                let e = match similar(&name.to_string(), names.iter().map(String::as_str)) {
                    Some(n) => e.help(format!("did you mean `{n}`?")),
                    None => e,
                };
                errors.push(e.note(format!(
                    "the {what} must be declared in the same `class!` block (a current \
                     limitation of roost: there are no imports between `class!` blocks)"
                )));
            }
            found
        };

        // Superclass chains.
        let mut chains = Vec::new();
        for (i, class) in classes.iter().enumerate() {
            let mut chain = vec![i];
            let mut cur = i;
            while let Some(parent) = &classes[cur].parent {
                let Some(p) = find(parent, errors, "superclass") else {
                    ok = false;
                    break;
                };
                if classes[p].is_interface() {
                    errors.push(
                        syn::Error::new(
                            parent.span(),
                            format!("`{parent}` is an interface, not a class"),
                        )
                        .help(format!("as in Java, a class `implements {parent}`")),
                    );
                    ok = false;
                    break;
                }
                if chain.contains(&p) {
                    errors.push(syn::Error::new(
                        class.name.span(),
                        format!("cyclic inheritance involving `{}`", class.name),
                    ));
                    ok = false;
                    break;
                }
                chain.push(p);
                cur = p;
            }
            chains.push(chain);
        }

        // Direct interface lists.
        let mut direct: Vec<Vec<usize>> = Vec::new();
        for class in &classes {
            let mut list = Vec::new();
            for name in &class.interfaces {
                let Some(j) = find(name, errors, "interface") else {
                    ok = false;
                    continue;
                };
                if !classes[j].is_interface() {
                    let e = if class.is_interface() {
                        syn::Error::new(
                            name.span(),
                            format!("`{name}` is a class: an interface can only extend interfaces"),
                        )
                        .help(format!(
                            "turn `{name}` into an `interface`, or remove it from this list"
                        ))
                    } else {
                        syn::Error::new(
                            name.span(),
                            format!("`{name}` is a class, not an interface"),
                        )
                        .help(format!("as in Java, a class `extends {name}`"))
                    };
                    errors.push(e);
                    ok = false;
                    continue;
                }
                if list.contains(&j) {
                    errors.push(syn::Error::new(
                        name.span(),
                        format!("`{name}` is listed twice"),
                    ));
                    ok = false;
                    continue;
                }
                list.push(j);
            }
            direct.push(list);
        }
        if !ok {
            return None;
        }

        // Interface closures (and cycles among interfaces).
        let mut ifaces = Vec::new();
        for (i, class) in classes.iter().enumerate() {
            let mut out: Vec<usize> = Vec::new();
            let mut stack: Vec<usize> = chains[i].iter().flat_map(|&c| direct[c].clone()).collect();
            stack.reverse();
            while let Some(j) = stack.pop() {
                if j == i {
                    errors.push(syn::Error::new(
                        class.name.span(),
                        format!("cyclic inheritance involving `{}`", class.name),
                    ));
                    ok = false;
                    break;
                }
                if !out.contains(&j) {
                    out.push(j);
                    stack.extend(direct[j].iter().rev());
                }
            }
            ifaces.push(out);
        }
        if !ok {
            return None;
        }

        let mut index = Index {
            classes,
            chains,
            ifaces,
            shared: Vec::new(),
        };
        let shared = Self::shared_groups(&index.classes, &index.chains, &index.ifaces, |t, m| {
            index.introduces(t, m)
        });
        index.shared = shared;
        index.check(errors);
        Some(index)
    }

    pub fn parent(&self, class: usize) -> Option<usize> {
        self.chains[class].get(1).copied()
    }

    /// Every class trait and interface trait type `t`'s trait has as a (transitive)
    /// supertrait, `t` included: its chain, then its interfaces.
    pub fn supertypes(&self, t: usize) -> impl Iterator<Item = usize> + '_ {
        self.chains[t].iter().chain(&self.ifaces[t]).copied()
    }

    /// The virtual method `name` declared (abstract or not) in type `t` itself.
    pub fn virtual_method(&self, t: usize, name: &Ident) -> Option<&Method> {
        self.classes[t]
            .methods
            .iter()
            .find(|m| m.is_virtual() && m.name() == name)
    }

    /// The class (starting at `from` and going up) that declares virtual method `name`.
    pub fn resolve_virtual(&self, from: usize, name: &Ident) -> Option<usize> {
        self.chains[from]
            .iter()
            .copied()
            .find(|&c| self.virtual_method(c, name).is_some())
    }

    /// Where `name` is declared for an override in class `c`: a superclass, an interface
    /// `c` implements, or `Object`.
    fn declared_above(&self, c: usize, name: &Ident) -> Option<Above> {
        if let Some(p) = self.parent(c)
            && let Some(owner) = self.resolve_virtual(p, name)
        {
            return Some(Above::Type(owner));
        }
        if let Some(&i) = self.ifaces[c]
            .iter()
            .find(|&&i| self.virtual_method(i, name).is_some())
        {
            return Some(Above::Type(i));
        }
        OBJECT_METHODS
            .iter()
            .any(|o| name == o)
            .then_some(Above::Object)
    }

    /// Does `t`'s trait declare `m` (as opposed to `m` overriding or implementing a method
    /// declared further up)?
    pub fn introduces(&self, t: usize, m: &Method) -> bool {
        m.is_virtual()
            && (self.classes[t].is_interface() || self.declared_above(t, m.name()).is_none())
    }

    /// The implementation of `name` that an object of class `c` runs (or that `super.name()`
    /// from a subclass of `c` reaches). `None`: abstract, or no such method.
    ///
    /// Java's rules: the nearest class wins (and an abstract declaration there means "not
    /// implemented"); then an interface `default`; then `Object`.
    pub fn resolve_impl(&self, c: usize, name: &Ident) -> Option<Impl> {
        for &k in &self.chains[c] {
            if let Some(m) = self.virtual_method(k, name) {
                return if m.is_abstract() {
                    None
                } else if self.classes[k].is_interface() {
                    Some(Impl::Default(k))
                } else {
                    Some(Impl::Class(k))
                };
            }
        }
        if let Some(&i) = self.ifaces[c].iter().find(|&&i| {
            self.virtual_method(i, name)
                .is_some_and(|m| !m.is_abstract())
        }) {
            return Some(Impl::Default(i));
        }
        OBJECT_METHODS
            .iter()
            .any(|o| name == o)
            .then_some(Impl::Object)
    }

    /// The shared-method group `t`'s declaration of `name` belongs to, if any.
    pub fn shared_of(&self, t: usize, name: &Ident) -> Option<&Shared> {
        self.shared
            .iter()
            .find(|g| g.name == *name && g.decls.contains(&t))
    }

    /// Groups of declarations of the same method by related types (types that some type
    /// has both of as supertypes). Each group becomes one trait, see [`Shared`].
    fn shared_groups(
        classes: &[Class],
        chains: &[Vec<usize>],
        ifaces: &[Vec<usize>],
        introduces: impl Fn(usize, &Method) -> bool,
    ) -> Vec<Shared> {
        let mut groups: Vec<Shared> = Vec::new();
        for t in 0..classes.len() {
            let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
            for d in chains[t].iter().chain(&ifaces[t]).copied() {
                for m in classes[d].methods.iter().filter(|m| introduces(d, m)) {
                    by_name.entry(m.name().to_string()).or_default().push(d);
                }
            }
            for (name, decls) in by_name.into_iter().filter(|(_, d)| d.len() > 1) {
                // Merge with every existing group of this name that shares a declaration.
                let mut merged = Shared {
                    name: Ident::new(&name, Span::call_site()),
                    decls,
                };
                groups.retain(|g| {
                    let overlaps =
                        g.name == merged.name && g.decls.iter().any(|d| merged.decls.contains(d));
                    if overlaps {
                        for &d in &g.decls {
                            if !merged.decls.contains(&d) {
                                merged.decls.push(d);
                            }
                        }
                    }
                    !overlaps
                });
                merged.decls.sort_unstable();
                groups.push(merged);
            }
        }
        groups
    }

    /// The field `this.name` refers to inside a method of `class`.
    pub fn resolve_field(&self, class: usize, name: &Ident) -> Result<FieldRef, String> {
        if self.classes[class].is_interface() {
            return Err(format!(
                "interface `{}` has no fields",
                self.classes[class].name
            ));
        }
        let mut private_in = None;
        for &c in &self.chains[class] {
            if let Some(f) = self.classes[c].fields.iter().find(|f| f.name == *name) {
                if f.is_static {
                    return Err(format!(
                        "`{name}` is a static field of `{0}`, not a field of `this`{HELP}write \
                         `{0}::{name}` or `Self::{name}` (Java's `{0}.{name}`)",
                        self.classes[c].name
                    ));
                }
                if c == class || f.access != Access::Private {
                    return Ok(FieldRef { owner: c });
                }
                private_in.get_or_insert(c);
            }
        }
        Err(match private_in {
            Some(c) => format!(
                "field `{name}` is private to class `{0}`{HELP}declare it `protected {name}` in \
                 `{0}` to use it in subclasses, as in Java",
                self.classes[c].name
            ),
            None => {
                let visible: Vec<String> = self.chains[class]
                    .iter()
                    .flat_map(|&c| {
                        self.classes[c]
                            .instance_fields()
                            .filter(move |f| c == class || f.access != Access::Private)
                    })
                    .map(|f| f.name.to_string())
                    .collect();
                let msg = format!("class `{}` has no field `{name}`", self.classes[class].name);
                match similar(&name.to_string(), visible.iter().map(String::as_str)) {
                    Some(f) => format!("{msg}{HELP}did you mean `this.{f}`?"),
                    None if visible.is_empty() => {
                        format!("{msg}{HELP}declare it in the class: `{name}: Type;`")
                    }
                    None => format!(
                        "{msg}{HELP}the fields `this` has here are: {}",
                        visible
                            .iter()
                            .map(|f| format!("`{f}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                }
            }
        })
    }

    /// The static field `Class::name` (`of` is `Class`), accessed from code in `from`.
    /// `None`: there is no static field of that name (it may be an associated function).
    pub fn resolve_static_field(
        &self,
        from: usize,
        of: usize,
        name: &Ident,
    ) -> Option<Result<FieldRef, String>> {
        for &c in &self.chains[of] {
            if let Some(f) = self.classes[c].static_fields().find(|f| f.name == *name) {
                return Some(if f.access == Access::Private && c != from {
                    Err(format!(
                        "static field `{name}` is private to class `{0}`{HELP}declare it \
                         `protected static {name}` in `{0}` to use it in subclasses",
                        self.classes[c].name
                    ))
                } else {
                    Ok(FieldRef { owner: c })
                });
            }
        }
        None
    }

    /// The class in `of`'s chain that declares static method `name`.
    pub fn resolve_static_method(&self, of: usize, name: &Ident) -> Option<usize> {
        self.chains[of].iter().copied().find(|&c| {
            self.classes[c]
                .methods
                .iter()
                .any(|m| m.is_static && m.name() == name)
        })
    }

    /// Private method `name` of `class`, if any.
    pub fn private_method(&self, class: usize, name: &Ident) -> Option<&Method> {
        self.classes[class]
            .methods
            .iter()
            .find(|m| m.access == Access::Private && !m.is_static && m.name() == name)
    }

    /// Every method name class `ci` inherits (superclasses, interfaces, `Object`), for
    /// "did you mean" suggestions.
    fn inherited_method_names(&self, ci: usize) -> Vec<String> {
        let mut names: Vec<String> = OBJECT_METHODS.iter().map(|m| (*m).to_string()).collect();
        for t in self.supertypes(ci).filter(|&t| t != ci) {
            for m in self.classes[t].methods.iter().filter(|m| m.is_virtual()) {
                names.push(m.name().to_string());
            }
        }
        names
    }

    pub fn find(&self, name: &Ident) -> Option<usize> {
        self.classes.iter().position(|c| c.name == *name)
    }

    fn above_name(&self, a: &Above) -> String {
        match a {
            Above::Type(t) => self.classes[*t].name.to_string(),
            Above::Object => "Object".to_string(),
        }
    }

    fn check(&self, errors: &mut Vec<syn::Error>) {
        for g in &self.shared {
            let sig = |t: usize| {
                let m = self.virtual_method(t, &g.name).expect("declared");
                let params: Vec<String> = m
                    .params()
                    .map(|p| p.ty.to_token_stream().to_string())
                    .collect();
                (m, params, m.sig.output.to_token_stream().to_string())
            };
            let (_, p0, r0) = sig(g.decls[0]);
            for &d in &g.decls[1..] {
                let (m, p, r) = sig(d);
                if (&p, &r) != (&p0, &r0) {
                    let (a, b) = (&self.classes[d].name, &self.classes[g.decls[0]].name);
                    let mut e = syn::Error::new(
                        m.name().span(),
                        format!(
                            "`{a}.{0}` and `{b}.{0}` must have the same signature: a class \
                             implements both with one method",
                            g.name
                        ),
                    )
                    .help(format!(
                        "make the parameter and return types of `{a}.{0}` and `{b}.{0}` identical \
                         (roost does not support Java's covariant return types)",
                        g.name
                    ));
                    let first = self.virtual_method(g.decls[0], &g.name).expect("declared");
                    e.combine(syn::Error::new(
                        first.name().span(),
                        format!("the other declaration of `{}`", g.name),
                    ));
                    errors.push(e);
                }
            }
        }
        for ci in 0..self.classes.len() {
            if self.classes[ci].is_interface() {
                self.check_interface(ci, errors);
            } else {
                self.check_class(ci, errors);
            }
        }
    }

    fn check_duplicates(&self, class: &Class, errors: &mut Vec<syn::Error>) {
        let mut err = |span: Span, msg: String| errors.push(error(span, &msg));
        for (i, f) in class.fields.iter().enumerate() {
            if class.fields[..i].iter().any(|g| g.name == f.name) {
                err(f.name.span(), format!("duplicate field `{}`", f.name));
            }
        }
        for (i, m) in class.methods.iter().enumerate() {
            let name = m.name();
            if class.methods[..i].iter().any(|n| n.name() == name) {
                err(
                    name.span(),
                    format!("duplicate method `{name}` (overloading is not supported)"),
                );
            }
        }
    }

    fn check_interface(&self, ii: usize, errors: &mut Vec<syn::Error>) {
        let iface = &self.classes[ii];
        self.check_duplicates(iface, errors);
        for m in &iface.methods {
            let name = m.name();
            if RESERVED.iter().chain(OBJECT_METHODS).any(|o| name == o) {
                errors.push(syn::Error::new(
                    name.span(),
                    format!("`{name}` is reserved (an `Object` method) and can't be declared in an interface"),
                ));
                continue;
            }
            if m.is_static {
                continue;
            }
            if let Some(&s) = self.ifaces[ii]
                .iter()
                .find(|&&s| self.virtual_method(s, name).is_some())
            {
                errors.push(syn::Error::new(
                    name.span(),
                    format!(
                        "`{name}` is already declared in super-interface `{}`: re-declaring or \
                         overriding it in a sub-interface is not supported",
                        self.classes[s].name
                    ),
                ));
            }
        }
    }

    fn check_class(&self, ci: usize, errors: &mut Vec<syn::Error>) {
        let class = &self.classes[ci];
        let mut err = |span: Span, msg: String| errors.push(error(span, &msg));
        if let (Some(parent), Some(p)) = (&class.parent, self.parent(ci))
            && self.classes[p].final_.is_some()
        {
            err(
                parent.span(),
                format!(
                    "cannot inherit from final class `{parent}`{HELP}remove `final` from `{parent}` \
                     if it is meant to be extended"
                ),
            );
        }
        self.check_duplicates(class, errors);
        let mut err = |span: Span, msg: String| errors.push(error(span, &msg));
        for m in &class.methods {
            let name = m.name();
            if RESERVED.iter().any(|o| name == o) {
                err(
                    name.span(),
                    format!("`{name}` is reserved (an `Object` method or the constructor)"),
                );
                continue;
            }
            if let Some(a) = m.abstract_
                && class.abstract_.is_none()
            {
                err(
                    a,
                    format!(
                        "abstract method `{name}` in class `{0}`, which is not abstract{HELP}\
                         declare `abstract class {0}`, or give the method a body",
                        class.name
                    ),
                );
            }
            let above = self.declared_above(ci, name);
            match (&above, m.is_virtual(), m.override_) {
                (None, _, Some(_)) => {
                    let inherited = self.inherited_method_names(ci);
                    let help =
                        match similar(&name.to_string(), inherited.iter().map(String::as_str)) {
                            Some(m) => format!("did you mean `{m}`?"),
                            None => "remove `@Override` if this is a new method".to_string(),
                        };
                    err(
                        name.span(),
                        format!(
                            "method `{name}` is marked @Override but does not override or \
                             implement a method from a superclass, an interface or \
                             `Object`{HELP}{help}"
                        ),
                    );
                }
                (Some(a), true, None) => err(
                    name.span(),
                    format!(
                        "method `{name}` {} `{}.{name}` but is not marked @Override{HELP}write \
                         `@Override` before the method: roost requires it (Java only recommends it)",
                        if matches!(a, Above::Type(t) if self.classes[*t].is_interface()) {
                            "implements"
                        } else {
                            "overrides"
                        },
                        self.above_name(a)
                    ),
                ),
                (Some(a), false, _) => err(
                    name.span(),
                    format!(
                        "{} method `{name}` would hide the inherited method `{}.{name}`",
                        if m.is_static { "static" } else { "private" },
                        self.above_name(a)
                    ),
                ),
                _ => {}
            }
            if let (Some(a), true, Some(s)) = (&above, m.is_virtual(), m.abstract_) {
                err(
                    s,
                    format!(
                        "abstract method `{name}` cannot re-declare `{}.{name}`: leave it out, \
                         an abstract class does not have to implement inherited methods",
                        self.above_name(a)
                    ),
                );
            }
            if m.is_virtual()
                && let Some(p) = self.parent(ci)
                && let Some(owner) = self.chains[p].iter().copied().find(|&c| {
                    self.virtual_method(c, name)
                        .is_some_and(|m| m.final_.is_some())
                })
            {
                err(
                    name.span(),
                    format!(
                        "cannot override final method `{0}.{name}`{HELP}remove `final` from \
                         `{0}.{name}`, or give this method another name",
                        self.classes[owner].name
                    ),
                );
            }
        }

        // A concrete class must end up with an implementation of every method it has.
        if class.abstract_.is_none() {
            let mut missing = Vec::new();
            let mut seen = Vec::new();
            for d in self.supertypes(ci) {
                for m in self.classes[d].methods.iter().filter(|m| m.is_virtual()) {
                    let name = m.name();
                    if seen.contains(name) {
                        continue;
                    }
                    seen.push(name.clone());
                    // An abstract method of this very class is reported on its own.
                    if self.resolve_impl(ci, name).is_none()
                        && self
                            .virtual_method(ci, name)
                            .is_none_or(|m| m.abstract_.is_none())
                    {
                        let owner = self.supertypes(ci).find(|&o| {
                            self.virtual_method(o, name)
                                .is_some_and(Method::is_abstract)
                        });
                        let owner = owner.unwrap_or(d);
                        missing.push(format!("`{}.{name}`", self.classes[owner].name));
                    }
                }
            }
            if !missing.is_empty() {
                err(
                    class.name.span(),
                    format!(
                        "class `{}` is not abstract and does not implement {}{HELP}implement {} \
                         (with @Override) or declare the class `abstract`",
                        class.name,
                        missing.join(", "),
                        if missing.len() == 1 { "it" } else { "them" },
                    ),
                );
            }
        }
    }
}

// Generated names. All of them start with `__roost_` / `__Roost` and are `#[doc(hidden)]`.

/// `trait AnimalClass` / `trait PetInterface`: the vtable; `type Animal = dyn AnimalClass`.
pub fn trait_name(class: &Class) -> Ident {
    if class.is_interface() {
        format_ident!("{}Interface", class.name)
    } else {
        format_ident!("{}Class", class.name)
    }
}

/// The concrete struct an `Animal` instance (exactly `Animal`, not a subclass) is.
pub fn struct_name(class: &Class) -> Ident {
    format_ident!("__Roost{}", class.name)
}

/// Struct field holding `owner`'s field `field`. Fields of different classes may share a name.
pub fn storage_name(owner: &Class, field: &Ident) -> Ident {
    format_ident!("__{}_{}", owner.name, field)
}

/// Trait method returning the `GcCell` of `owner`'s field `field`.
pub fn accessor_name(owner: &Class, field: &Ident) -> Ident {
    format_ident!("__roost_field_{}_{}", owner.name, field)
}

/// The thread-local holding `owner`'s static field `field`.
pub fn static_name(owner: &Class, field: &Ident) -> Ident {
    format_ident!("__ROOST_STATIC_{}_{}", owner.name, field)
}

/// `impl dyn AnimalClass { fn __roost_method_speak<T>(this: &T) }`: Animal's implementation.
pub fn method_impl_name(name: &Ident) -> Ident {
    format_ident!("__roost_method_{}", name)
}

pub fn private_impl_name(name: &Ident) -> Ident {
    format_ident!("__roost_private_{}", name)
}

/// The trait holding a [`Shared`] method: named after its first declaration.
pub fn shared_trait_name(index: &Index, g: &Shared) -> Ident {
    format_ident!("__Roost{}_{}", index.classes[g.decls[0]].name, g.name)
}

/// The marker trait whose unsatisfiable bound makes `Shape::new(..)` an error.
pub fn abstract_marker_name(class: &Class) -> Ident {
    format_ident!("__RoostAbstract{}", class.name)
}

pub const CONSTRUCT: &str = "__roost_construct";
