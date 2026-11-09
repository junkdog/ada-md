//! The theme's `[syntax]` rules as scope selectors, and the rule a scope stack takes, picked as
//! syntect 5.3 picked it (`Highlighter::style_for_stack`) when ada highlighted with syntect:
//! user themes were written against that, and ada's lexers' kinds carry bat's scopes so that
//! they still apply (ADR-07, ADR-40, ADR-43). Tests hold it to syntect's on every kind of every
//! lexer, in every built-in theme.
//!
//! A rule's selectors are split at `,` and `|`; each is a path of scopes, then any number of
//! ` -` and a path to exclude. A selector of one scope and no exclusion is a *single*: it is
//! tried against the stack's every scope, its score the scope's atoms times 8 to the power of
//! that scope's depth, so the deepest and then the longest match wins. Any other is tried
//! against the whole stack, its path's scopes matched in order, each as early as it can be,
//! scored the same way, summed; one of no path scores 1. The rule with the highest score takes
//! the stack: among singles of equal score, the one with more atoms, then the one first in the
//! theme; a selector of the others only when it scores higher than everything before it.

use super::Rule;

/// syntect's `ATOM_LEN_BITS`: each scope deeper in the stack weighs 2³ times as much.
const ATOM_LEN_BITS: u16 = 3;

/// A scope as a selector names it: its atoms, at most eight, split at `.`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Scope {
    /// Without a trailing `.`; empty for the empty scope, a prefix of every scope.
    text: Box<str>,
    atoms: u32,
}

impl Scope {
    /// syntect's `Scope::new`: trimmed, trailing dots dropped, `None` past eight atoms.
    fn new(s: &str) -> Option<Self> {
        let text = s.trim().trim_end_matches('.');
        let atoms = if text.is_empty() {
            0
        } else {
            text.split('.').count() as u32
        };
        (atoms <= 8).then(|| Scope {
            text: text.into(),
            atoms,
        })
    }

    /// Whether this scope's atoms begin `scope`'s.
    fn is_prefix_of(&self, scope: &str) -> bool {
        let t = &*self.text;
        t.is_empty()
            || scope
                .strip_prefix(t)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    }

    /// The score of a match on the stack's scope `depth`.
    fn score(&self, depth: usize) -> f64 {
        f64::from(self.atoms) * f64::from(ATOM_LEN_BITS * depth as u16).exp2()
    }
}

/// Scopes matched against a stack in order, each at the first scope it can be: syntect's
/// `ScopeStack::does_match`.
fn path_score(path: &[Scope], stack: &[&str]) -> Option<f64> {
    let mut next = path.iter();
    let mut want = next.next()?;
    let mut score = 0.0;
    for (depth, scope) in stack.iter().enumerate() {
        if want.is_prefix_of(scope) {
            score += want.score(depth);
            match next.next() {
                Some(w) => want = w,
                None => return Some(score),
            }
        }
    }
    None
}

/// One selector of a rule: a path, and the paths that rule a stack out.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Selector {
    path: Vec<Scope>,
    excludes: Vec<Vec<Scope>>,
}

impl Selector {
    /// syntect's `ScopeSelector::from_str`.
    fn parse(s: &str) -> Option<Self> {
        let mut parts = s.split(" -");
        let path = stack(parts.next().unwrap_or_default())?;
        let excludes = parts.map(stack).collect::<Option<_>>()?;
        Some(Selector { path, excludes })
    }

    /// syntect's `ScopeSelector::does_match`.
    fn score(&self, stack: &[&str]) -> Option<f64> {
        let out = |ex: &Vec<Scope>| ex.is_empty() || path_score(ex, stack).is_some();
        if self.excludes.iter().any(out) {
            return None;
        }
        if self.path.is_empty() {
            return Some(1.0);
        }
        path_score(&self.path, stack)
    }
}

/// Scopes separated by blanks.
fn stack(s: &str) -> Option<Vec<Scope>> {
    s.split_whitespace().map(Scope::new).collect()
}

/// A theme's `[syntax]` rules as selectors, and their rules.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selectors {
    /// Selectors of one scope and no exclusion, the ones of more atoms first, then in the
    /// theme's order.
    singles: Vec<(Scope, Rule)>,
    /// The rest, in the theme's order.
    others: Vec<(Selector, Rule)>,
}

impl Selectors {
    /// The selectors of `rules`, rule `i` the `i`th; a rule whose selectors don't parse (a
    /// scope of over eight atoms) is left out.
    pub fn new<'a>(rules: impl IntoIterator<Item = &'a str>) -> Self {
        let mut out = Selectors::default();
        for (i, rule) in rules.into_iter().enumerate() {
            let Some(selectors) = rule
                .split([',', '|'])
                .map(Selector::parse)
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let rule = Rule(u16::try_from(i).expect("rules fit a u16"));
            for mut sel in selectors {
                if sel.path.len() == 1 && sel.excludes.is_empty() {
                    out.singles.push((sel.path.remove(0), rule));
                } else {
                    out.others.push((sel, rule));
                }
            }
        }
        // stable: equal lengths keep the theme's order
        out.singles
            .sort_by_key(|(scope, _)| std::cmp::Reverse(scope.atoms));
        out
    }

    /// The rule `stack` takes, its outermost scope first; [`Rule::NONE`] when none matches.
    pub fn rule(&self, stack: &[&str]) -> Rule {
        let mut best = (-1.0, Rule(Rule::NONE));
        let mut take = |score: f64, rule: Rule| {
            if score > best.0 {
                best = (score, rule);
            }
        };
        for (depth, scope) in stack.iter().enumerate() {
            for (single, rule) in &self.singles {
                if single.is_prefix_of(scope) {
                    take(single.score(depth), *rule);
                }
            }
        }
        for (sel, rule) in &self.others {
            if let Some(score) = sel.score(stack) {
                take(score, *rule);
            }
        }
        best.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(rules: &[&str], stack: &str) -> u16 {
        let stack: Vec<&str> = stack.split_whitespace().collect();
        Selectors::new(rules.iter().copied()).rule(&stack).0
    }

    #[test]
    fn the_deepest_then_the_longest_match_wins() {
        let rules = ["string", "string.quoted", "source.rust", "comment"];
        assert_eq!(rule(&rules, "source.rust string.quoted.double"), 1);
        assert_eq!(rule(&rules, "source.rust string.unquoted"), 0);
        assert_eq!(rule(&rules, "source.rust"), 2);
        assert_eq!(rule(&rules, "source.rust meta.block"), 2);
        assert_eq!(rule(&rules, "text.plain"), Rule::NONE);
        // an atom, not a piece of one
        assert_eq!(rule(&["str"], "source.rust string"), Rule::NONE);
    }

    #[test]
    fn paths_exclusions_and_unions() {
        let rules = [
            "string - string.regexp",
            "source.rust meta.function entity.name",
            "keyword | storage",
        ];
        assert_eq!(rule(&rules, "source.rust string.quoted"), 0);
        assert_eq!(rule(&rules, "source.rust string.regexp"), Rule::NONE);
        assert_eq!(
            rule(&rules, "source.rust meta.function entity.name.function"),
            1
        );
        assert_eq!(rule(&rules, "source.rust entity.name.function"), Rule::NONE);
        assert_eq!(rule(&rules, "source.rust storage.type"), 2);
    }

    /// syntect's own examples of its scores (`selector.rs`, `scope.rs`).
    #[test]
    fn scores_are_syntects() {
        let score = |sel: &str, stack: &str| {
            let stack: Vec<&str> = stack.split_whitespace().collect();
            let sels: Vec<Selector> = sel
                .split([',', '|'])
                .map(|s| Selector::parse(s).unwrap())
                .collect();
            sels.iter().filter_map(|s| s.score(&stack)).reduce(f64::max)
        };
        assert_eq!(score("a.b, a e, e.f", "a.b e.f"), Some(f64::from(0o20)));
        assert_eq!(score("a.b, a e.f, e.f", "a.b e.f"), Some(f64::from(0o21)));
        assert_eq!(
            score("a.b, a e.f - c k, e.f - a.b", "a.b c.d j e.f"),
            Some(f64::from(0o2001))
        );
        assert_eq!(score("a.b c e.f", "a.b c.d e.f.g"), Some(f64::from(0o212)));
        assert_eq!(score("a c.d.e", "a.b c.d e.f.g"), None);
        assert_eq!(score(" -a.b", "x"), Some(1.0));
        assert_eq!(score(" -a.b", "a.b.c"), None);
    }

    #[test]
    fn a_scope_of_over_eight_atoms_drops_its_rule() {
        let rules = ["a.b.c.d.e.f.g.h.i, string", "string"];
        assert_eq!(rule(&rules, "source string"), 1);
        // trailing dots and blanks are trimmed, as syntect trims them
        assert_eq!(rule(&["string. "], "source string.quoted"), 0);
    }
}
