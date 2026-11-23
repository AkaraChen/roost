//! Name resolution over a parsed [`Hierarchy`]: superclass chains, which class a
//! `this.field` / `super.m()` refers to, and the semantic checks (`@Override`, ...).

use proc_macro2::Span;
use quote::format_ident;
use syn::Ident;

use crate::model::{Access, Class, Hierarchy, Method};

/// Methods every object already has (from `roost::Object`). Overriding them is a later step.
const OBJECT_METHODS: &[&str] = &[
    "to_string",
    "equals",
    "hash_code",
    "class_name",
    "as_any",
    "new",
];

pub struct Index {
    pub classes: Vec<Class>,
    /// `chains[i]`: class `i`, its parent, ..., the root (indices into `classes`).
    pub chains: Vec<Vec<usize>>,
}

/// A field as seen from some class: who declares it.
pub struct FieldRef {
    pub owner: usize,
}

impl Index {
    pub fn build(h: Hierarchy, errors: &mut Vec<syn::Error>) -> Option<Index> {
        let classes = h.classes;
        let mut ok = true;
        for (i, c) in classes.iter().enumerate() {
            if let Some(prev) = classes[..i].iter().find(|p| p.name == c.name) {
                let mut e = syn::Error::new(
                    c.name.span(),
                    format!("class `{}` is declared twice", c.name),
                );
                e.combine(syn::Error::new(prev.name.span(), "first declared here"));
                errors.push(e);
                ok = false;
            }
        }
        let mut chains = Vec::new();
        for (i, _) in classes.iter().enumerate() {
            let mut chain = vec![i];
            let mut cur = i;
            while let Some(parent) = &classes[cur].parent {
                let Some(p) = classes.iter().position(|c| c.name == *parent) else {
                    errors.push(syn::Error::new(
                        parent.span(),
                        format!(
                            "unknown superclass `{parent}`: the superclass must be declared in the same \
                             `class!` block (a current limitation of roost)"
                        ),
                    ));
                    ok = false;
                    break;
                };
                if chain.contains(&p) {
                    errors.push(syn::Error::new(
                        classes[i].name.span(),
                        format!("cyclic inheritance involving `{}`", classes[i].name),
                    ));
                    ok = false;
                    break;
                }
                chain.push(p);
                cur = p;
            }
            chains.push(chain);
        }
        if !ok {
            return None;
        }
        let index = Index { classes, chains };
        index.check(errors);
        Some(index)
    }

    pub fn parent(&self, class: usize) -> Option<usize> {
        self.chains[class].get(1).copied()
    }

    /// The class (starting at `from` and going up) whose virtual method `name` is in effect.
    pub fn resolve_virtual(&self, from: usize, name: &Ident) -> Option<usize> {
        self.chains[from].iter().copied().find(|&c| {
            self.classes[c]
                .methods
                .iter()
                .any(|m| m.is_virtual() && m.name() == name)
        })
    }

    /// Does `class` introduce `m` (as opposed to overriding an inherited method)?
    pub fn introduces(&self, class: usize, m: &Method) -> bool {
        m.is_virtual()
            && self
                .parent(class)
                .is_none_or(|p| self.resolve_virtual(p, m.name()).is_none())
    }

    /// The field `this.name` refers to inside a method of `class`.
    pub fn resolve_field(&self, class: usize, name: &Ident) -> Result<FieldRef, String> {
        let mut private_in = None;
        for &c in &self.chains[class] {
            if let Some(f) = self.classes[c].fields.iter().find(|f| f.name == *name) {
                if c == class || f.access != Access::Private {
                    return Ok(FieldRef { owner: c });
                }
                private_in.get_or_insert(c);
            }
        }
        Err(match private_in {
            Some(c) => format!(
                "field `{name}` is private to class `{}`",
                self.classes[c].name
            ),
            None => format!("class `{}` has no field `{name}`", self.classes[class].name),
        })
    }

    /// Private method `name` of `class`, if any.
    pub fn private_method(&self, class: usize, name: &Ident) -> Option<&Method> {
        self.classes[class]
            .methods
            .iter()
            .find(|m| m.access == Access::Private && !m.is_static && m.name() == name)
    }

    fn check(&self, errors: &mut Vec<syn::Error>) {
        for (ci, class) in self.classes.iter().enumerate() {
            let mut err = |span: Span, msg: String| errors.push(syn::Error::new(span, msg));
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
                    continue;
                }
                if OBJECT_METHODS.iter().any(|o| name == o) {
                    err(
                        name.span(),
                        format!("`{name}` is reserved (an `Object` method or the constructor)"),
                    );
                    continue;
                }
                let inherited = self.parent(ci).and_then(|p| self.resolve_virtual(p, name));
                match (inherited, m.is_virtual(), m.override_) {
                    (None, _, Some(span)) => err(
                        span,
                        format!("method `{name}` does not override a method from a superclass"),
                    ),
                    (Some(owner), true, None) => err(
                        name.span(),
                        format!(
                            "method `{name}` overrides `{}.{name}` but is not marked @Override",
                            self.classes[owner].name
                        ),
                    ),
                    (Some(owner), false, _) => err(
                        name.span(),
                        format!(
                            "{} method `{name}` would hide the inherited method `{}.{name}`",
                            if m.is_static { "static" } else { "private" },
                            self.classes[owner].name
                        ),
                    ),
                    _ => {}
                }
            }
        }
    }
}

// Generated names. All of them start with `__roost_` / `__Roost` and are `#[doc(hidden)]`.

/// `trait AnimalClass`: the vtable; `type Animal = dyn AnimalClass`.
pub fn trait_name(class: &Class) -> Ident {
    format_ident!("{}Class", class.name)
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

/// `impl dyn AnimalClass { fn __roost_method_speak<T>(this: &T) }`: Animal's implementation.
pub fn method_impl_name(name: &Ident) -> Ident {
    format_ident!("__roost_method_{}", name)
}

pub fn private_impl_name(name: &Ident) -> Ident {
    format_ident!("__roost_private_{}", name)
}

pub const CONSTRUCT: &str = "__roost_construct";
