//! Error reporting with `help:` / `note:` lines.
//!
//! Errors travel as `syn::Error`s everywhere in this crate. A help or note is appended to the
//! message behind a marker ([`Help::help`]); [`emit`] turns each error into a real rustc
//! diagnostic (`proc_macro::Diagnostic`, nightly, which roost requires anyway), so the user sees
//!
//! ```text
//! error: unknown superclass `Animl`
//!  --> src/main.rs:5:30
//!   |
//! 5 |     public class Dog extends Animl {}
//!   |                              ^^^^^
//!   |
//!   = help: did you mean `Animal`?
//! ```
//!
//! Outside a proc-macro invocation (the unit tests) there is no compiler to talk to: the errors
//! become `compile_error!`s with the help lines appended to the message.

use proc_macro2::TokenStream;

/// Separates the main message from each help / note line. Not something a user can type.
const MARK: char = '\u{0}';

pub trait Help: Sized {
    /// Adds a `= help: ..` line.
    fn help(self, msg: impl std::fmt::Display) -> Self;
    /// Adds a `= note: ..` line.
    fn note(self, msg: impl std::fmt::Display) -> Self;
}

impl Help for syn::Error {
    fn help(self, msg: impl std::fmt::Display) -> Self {
        append(self, "help", msg)
    }

    fn note(self, msg: impl std::fmt::Display) -> Self {
        append(self, "note", msg)
    }
}

/// Appends to the last message of `e` (the one most recently combined in).
fn append(e: syn::Error, level: &str, msg: impl std::fmt::Display) -> syn::Error {
    let mut parts: Vec<syn::Error> = e.into_iter().collect();
    let last = parts.pop().expect("a syn::Error has at least one message");
    let mut out = syn::Error::new(last.span(), format!("{last}{MARK}{level}{MARK}{msg}"));
    if let Some(mut first) = parts.into_iter().reduce(|mut a, b| {
        a.combine(b);
        a
    }) {
        first.combine(out);
        out = first;
    }
    out
}

/// `msg` with its help / note lines split off: `(message, [(level, text)])`.
fn split(msg: &str) -> (&str, Vec<(&str, &str)>) {
    let mut it = msg.split(MARK);
    let main = it.next().unwrap_or_default();
    let mut extra = Vec::new();
    while let (Some(level), Some(text)) = (it.next(), it.next()) {
        extra.push((level, text));
    }
    (main, extra)
}

/// Reports `errors`. Inside a proc-macro invocation they are emitted as compiler diagnostics
/// and the returned stream is empty; otherwise (unit tests) it is a list of `compile_error!`s.
pub fn emit<'a>(errors: impl IntoIterator<Item = &'a syn::Error>) -> TokenStream {
    let mut out = TokenStream::new();
    for error in errors {
        for e in error.clone() {
            let text = e.to_string();
            let (main, extra) = split(&text);
            if proc_macro::is_available() {
                use proc_macro::{Diagnostic, Level};
                let mut d = Diagnostic::spanned(e.span().unwrap(), Level::Error, main);
                for (level, text) in extra {
                    d = if level == "help" {
                        d.help(text)
                    } else {
                        d.note(text)
                    };
                }
                d.emit();
            } else {
                let mut msg = main.to_string();
                for (level, text) in extra {
                    msg.push_str(&format!("\n= {level}: {text}"));
                }
                out.extend(syn::Error::new(e.span(), msg).to_compile_error());
            }
        }
    }
    out
}

/// The closest of `candidates` to `name` (case-insensitive edit distance), if close enough to
/// be a typo: "did you mean `Animal`?".
pub fn similar<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let lower = name.to_lowercase();
    // One edit per three characters: `nmae` -> `name`, but `g` is not a typo of `f`.
    let limit = name.chars().count() / 3;
    candidates
        .into_iter()
        .filter(|c| *c != name)
        .map(|c| (distance(&lower, &c.to_lowercase()), c))
        .filter(|&(d, _)| d <= limit)
        .min_by_key(|&(d, _)| d)
        .map(|(_, c)| c)
}

/// Optimal string alignment distance (Levenshtein + adjacent transpositions): `pubilc` is one
/// edit away from `public`.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typos_are_found() {
        assert_eq!(distance("pubilc", "public"), 1);
        assert_eq!(similar("Animl", ["Animal", "Dog"]), Some("Animal"));
        assert_eq!(similar("nmae", ["name", "age"]), Some("name"));
        assert_eq!(similar("x", ["name"]), None);
        assert_eq!(similar("g", ["f"]), None);
        assert_eq!(similar("Dog", ["Dog"]), None);
    }

    #[test]
    fn help_lines_survive_combine() {
        let mut e = syn::Error::new(proc_macro2::Span::call_site(), "first");
        e.combine(syn::Error::new(proc_macro2::Span::call_site(), "second"));
        let e = e.help("try this");
        let msgs: Vec<String> = e.into_iter().map(|e| e.to_string()).collect();
        assert_eq!(msgs[0], "first");
        assert_eq!(split(&msgs[1]), ("second", vec![("help", "try this")]));
    }
}
