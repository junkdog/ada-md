//! What XML's scanner (`xml_scan.rs`) keeps from bat's `XML`: the scopes, and the matchers of
//! the regexes it tests at a position, for what isn't plain. Names' `\p{L}`
//! and `\p{N}` are ASCII letters and digits here, and any byte past ASCII, so that a name never
//! ends inside a character; on ASCII text the scopes are syntect's.

use crate::matcher::{Line, M, atoms, is_space, is_word};

atoms! {
    CDATA_META = "meta.tag.sgml.cdata.xml",
    CDATA_TEXT = "string.unquoted.cdata.xml",
    TAG_END = "punctuation.definition.tag.end.xml",
    COMMENT_BLOCK = "comment.block.xml",
    COMMENT_END = "punctuation.definition.comment.end.xml",
    DOUBLE_HYPHEN = "invalid.illegal.double-hyphen-within-comment.xml",
    GROUP_ENUMERATED = "meta.group.enumerated.xml",
    GROUP_END = "punctuation.definition.group.end.xml",
    LOGICAL_SEP = "punctuation.separator.logical.xml",
    OPERATOR = "keyword.operator.xml",
    BRACKETS = "meta.brackets.xml",
    INTERNAL_SUBSET = "meta.internal-subset.xml",
    BRACKETS_END = "punctuation.section.brackets.end.xml",
    UNKNOWN_META = "meta.tag.sgml.unknown.xml",
    PREPROCESSOR_META = "meta.tag.preprocessor.xml",
    BAD_ATTR_VALUE = "invalid.illegal.bad-attribute-value.xml",
    TAG_META = "meta.tag.xml",
    BAD_TAG_END = "invalid.illegal.bad-tag-end.xml",
    UNEXPECTED_ATTR = "invalid.illegal.unexpected-attribute.xml",
    TAG_BEGIN = "punctuation.definition.tag.begin.xml",
    CDATA_KW = "keyword.declaration.cdata.xml",
    COMMENT_BEGIN = "punctuation.definition.comment.begin.xml",
    DOCTYPE_KW = "keyword.declaration.doctype.xml",
    BAD_TAG_NAME = "invalid.illegal.bad-tag-name.xml",
    DOCTYPE_META_S = "meta.tag.sgml.doctype.xml",
    ROOT_NS = "variable.other.documentroot.namespace.xml",
    ROOT = "variable.other.documentroot.xml",
    NS_SEP = "punctuation.separator.namespace.xml",
    ROOT_LOCAL = "variable.other.documentroot.localname.xml",
    ATTLIST_KW = "keyword.declaration.attlist.xml",
    ATTR_TYPE = "storage.type.attribute.xml",
    ATTR_DEFAULT = "storage.modifier.attribute.default-value.xml",
    STORAGE_MARK = "punctuation.definition.storage.xml",
    ATTR_NAME = "entity.other.attribute-name.xml",
    ATTLIST_META = "meta.tag.sgml.attlist.xml",
    GROUP_BEGIN = "punctuation.definition.group.begin.xml",
    PARAMETER = "variable.parameter.xml",
    PARAMETER_MARK = "punctuation.definition.parameter.xml",
    PARAMETER_END = "punctuation.terminator.parameter.xml",
    BAD_IDENTIFIER = "invalid.illegal.bad-identifier.xml",
    EXTERNAL = "storage.type.external-content.xml",
    ELEMENT_KW = "keyword.declaration.element.xml",
    CONSTANT = "constant.other.xml",
    ELEMENT_META = "meta.tag.sgml.element.xml",
    ELEMENT_NAME = "variable.other.element.xml",
    GROUP = "meta.group.xml",
    SEPARATOR = "punctuation.separator.xml",
    MISSING_TAG_END = "invalid.illegal.missing-tag-end.xml",
    UNEXPECTED = "invalid.illegal.unexpected.xml",
    ENTITY_KW = "keyword.declaration.entity.xml",
    NDATA = "storage.type.ndata.xml",
    ENTITY_META = "meta.tag.sgml.entity.xml",
    ENTITY_NAME = "variable.other.entity.xml",
    ENTITY_MARK = "punctuation.definition.entity.xml",
    NOTATION_KW = "keyword.declaration.notation.xml",
    NOTATION_META = "meta.tag.sgml.notation.xml",
    NOTATION_NAME = "variable.other.notation.xml",
    BRACKETS_BEGIN = "punctuation.section.brackets.begin.xml",
    SUBSET_META = "meta.tag.sgml.subset.xml",
    SUBSET_NAME = "variable.other.subset.xml",
    ENTITY_HEX = "constant.character.entity.hexadecimal.xml",
    ENTITY_END = "punctuation.terminator.entity.xml",
    ENTITY_DEC = "constant.character.entity.decimal.xml",
    ENTITY_NAMED = "constant.character.entity.named.xml",
    PLACEHOLDER = "constant.other.placeholder.xml",
    CONSTANT_MARK = "punctuation.definition.constant.xml",
    TAG_NAME = "entity.name.tag.xml",
    BAD_AMPERSAND = "invalid.illegal.bad-ampersand.xml",
    MISSING_ENTITY = "invalid.illegal.missing-entity.xml",
    STR_BEGIN = "punctuation.definition.string.begin.xml",
    DOUBLE = "string.quoted.double.xml",
    STR_END = "punctuation.definition.string.end.xml",
    SINGLE = "string.quoted.single.xml",
    UNQUOTED = "string.unquoted.xml",
    TAG_NS = "entity.name.tag.namespace.xml",
    TAG_LOCAL = "entity.name.tag.localname.xml",
    ATTR_NS = "entity.other.attribute-name.namespace.xml",
    BAD_ATTR_NAME = "invalid.illegal.bad-attribute-name.xml",
    ATTR_LOCAL = "entity.other.attribute-name.localname.xml",
    KEY_VALUE = "punctuation.separator.key-value.xml",
}

// the matchers: each finds its regex's leftmost match at or after `from`

/// `[\p{L}_]`
pub(crate) fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

/// `[\p{L}\p{N}_.-]`
pub(crate) fn is_name(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-') || c >= 0x80
}

/// `[\p{L}:_]`
pub(crate) fn is_name2_start(c: u8) -> bool {
    is_name_start(c) || c == b':'
}

/// `[\p{L}\p{N}:_.-]`
pub(crate) fn is_name2(c: u8) -> bool {
    is_name(c) || c == b':'
}

/// `\b` at `i`.
pub(crate) fn boundary(l: &Line, i: usize) -> bool {
    let w = |i: usize| l.is(i, |c| is_word(c) && c != b'\n');
    (i > 0 && w(i - 1)) != w(i)
}

/// [`doctype`] at `k`, where a `<!` is.
pub(crate) fn doctype_at(l: &Line, k: usize) -> Option<M> {
    let word = l.b.get(k + 2..k + 9)?;
    if !(word.eq_ignore_ascii_case(b"DOCTYPE") && boundary(l, k + 9)) {
        return None;
    }
    let group = if word == b"DOCTYPE" { 2 } else { 3 };
    Some(M::new(k, k + 9).g(1, k, k + 2).g(group, k + 2, k + 9))
}

/// [`declaration`] at `k`, where a `<!` is.
pub(crate) fn declaration_at(l: &Line, k: usize, kw: &[u8]) -> Option<M> {
    let e = k + 2 + kw.len();
    (l.b.get(k + 2..e) == Some(kw) && boundary(l, e))
        .then(|| M::new(k, e).g(1, k, k + 2).g(2, k + 2, e))
}

/// [`unknown_open`] at `k`, where a `<!` is.
pub(crate) fn unknown_at(l: &Line, k: usize) -> M {
    let e = l.run(k + 2, |c| {
        !matches!(c, b'?' | b'/' | b'<' | b'>') && !is_space(c)
    });
    M::new(k, e).g(1, k, k + 2).g(2, k + 2, e)
}

/// [`attr_type`] at `p`.
pub(crate) fn attr_type_at(l: &Line, p: usize) -> Option<M> {
    const WORDS: [&[u8]; 9] = [
        b"CDATA",
        b"ENTITY",
        b"ENTITIES",
        b"IDREFS",
        b"IDREF",
        b"ID",
        b"NMTOKENS",
        b"NMTOKEN",
        b"NOTATION",
    ];
    if !boundary(l, p) {
        return None;
    }
    let rest = l.b.get(p..)?;
    WORDS
        .iter()
        .find(|w| rest.starts_with(w) && boundary(l, p + w.len()))
        .map(|w| M::new(p, p + w.len()))
}

/// [`attr_default`] at `k`, where a `#` is.
pub(crate) fn attr_default_at(l: &Line, k: usize) -> Option<M> {
    let rest = &l.b[k + 1..];
    [&b"FIXED"[..], b"IMPLIED", b"REQUIRED"]
        .into_iter()
        .find(|w| rest.starts_with(w) && boundary(l, k + 1 + w.len()))
        .map(|w| M::new(k, k + 1 + w.len()).g(1, k, k + 1))
}

/// [`dtd_attr_name`] at `p`.
pub(crate) fn dtd_attr_name_at(l: &Line, p: usize) -> Option<M> {
    if !(l.is(p, is_name2_start) && boundary(l, p)) {
        return None;
    }
    let run = l.run(p + 1, is_name2);
    (p + 1..=run)
        .rev()
        .find(|&e| boundary(l, e))
        .map(|e| M::new(p, e))
}

/// [`placeholder`] at `k`, where a `#` is.
pub(crate) fn placeholder_at(l: &Line, k: usize) -> Option<M> {
    let rest = &l.b[k + 1..];
    if rest.starts_with(b"PCDATA") {
        return Some(M::new(k, k + 7).g(1, k, k + 1));
    }
    rest.starts_with(b"CDATA")
        .then(|| M::new(k, k + 6).g(1, k, k + 1))
}

/// [`marked_name`] at `k`, where the mark is.
pub(crate) fn marked_name_at(l: &Line, k: usize) -> Option<M> {
    if !l.is(k + 1, is_name2_start) {
        return None;
    }
    let e = l.run(k + 2, is_name2);
    (l.at(e) == Some(b';')).then(|| M::new(k, e + 1).g(1, k, k + 1).g(2, e, e + 1))
}

/// [`char_ref`] at `k`, where a `&#` is.
pub(crate) fn char_ref_at(l: &Line, k: usize, hex: bool) -> Option<M> {
    let x = usize::from(hex);
    let marked = !hex || l.is(k + 2, |c| c == b'x' || c == b'X');
    let digit = |c: u8| {
        if hex {
            c.is_ascii_hexdigit()
        } else {
            c.is_ascii_digit()
        }
    };
    if !(marked && l.is(k + 2 + x, digit)) {
        return None;
    }
    let e = l.run(k + 2 + x, digit);
    (l.at(e) == Some(b';')).then(|| M::new(k, e + 1).g(1, k, k + 2 + x).g(2, e, e + 1))
}

/// [`dtd_name`] at `p`.
pub(crate) fn dtd_name_at(l: &Line, p: usize) -> Option<M> {
    if !l.is(p, is_name2_start) {
        return None;
    }
    let e = l.run(p + 1, is_name2);
    l.is(e, |c| b"'\"[]()<>".contains(&c) || is_space(c))
        .then(|| M::new(p, e))
}

/// [`preprocessor_xml`] at `k`, where a `<?` is.
pub(crate) fn preprocessor_xml_at(l: &Line, k: usize) -> Option<M> {
    let ahead = |i: usize| l.is(i, |c| matches!(c, b'?' | b'<' | b'>') || is_space(c));
    let q = k + 2;
    if l.b.get(q..q + 3) == Some(b"xml") {
        let mut e = q + 3;
        if l.at(e) == Some(b'-') && l.is(e + 1, |c| c == b'_' || c.is_ascii_lowercase()) {
            e = l.run(e + 2, |c| {
                matches!(c, b'-' | b'_') || c.is_ascii_lowercase() || c.is_ascii_digit()
            });
        }
        if ahead(e) {
            return Some(M::new(k, e).g(1, k, q).g(2, q, e));
        }
    }
    if l.b
        .get(q..q + 3)
        .is_some_and(|w| w.eq_ignore_ascii_case(b"xml"))
    {
        let e = l.run(q + 3, |c| !matches!(c, b'?' | b'<' | b'>') && !is_space(c));
        return Some(M::new(k, e).g(1, k, q).g(3, q, e));
    }
    None
}

/// [`preprocessor_other`] at `k`, where a `<?` is.
pub(crate) fn preprocessor_other_at(l: &Line, k: usize) -> Option<M> {
    let q = k + 2;
    if !l.is(q, is_name2_start) {
        return None;
    }
    let run = l.run(q + 1, is_name2);
    (q + 1..=run)
        .rev()
        .find(|&e| boundary(l, e))
        .map(|e| M::new(k, e).g(1, k, q).g(2, q, e))
}

/// A qualified name at `q`, as tags and the document root have it:
/// `(?:(?:(N)|([^?!/<>\s][^:/<>\s]*))(:))?(?:(N)(?=[/<>\s])|([^?!/<>\s][^/<>\s]*))`, with `N`
/// `[\p{L}_][\p{L}\p{N}_.-]*`. Its end, and its five groups (from `first`) set on `m`.
pub(crate) fn qname(l: &Line, q: usize, mut m: M, first: usize) -> Option<M> {
    let lead = |c: u8| !matches!(c, b'?' | b'!' | b'/' | b'<' | b'>') && !is_space(c);
    let local = |at: usize| -> Option<(usize, bool)> {
        if l.is(at, is_name_start) {
            let e = l.run(at + 1, is_name);
            if l.is(e, |c| matches!(c, b'/' | b'<' | b'>') || is_space(c)) {
                return Some((e, true));
            }
        }
        l.is(at, lead).then(|| {
            let e = l.run(at + 1, |c| !matches!(c, b'/' | b'<' | b'>') && !is_space(c));
            (e, false)
        })
    };
    // the namespace, when one and a local name after it match
    let ns = if l.is(q, is_name_start) {
        let e = l.run(q + 1, is_name);
        (l.at(e) == Some(b':')).then_some((e, true))
    } else {
        None
    };
    let ns = ns.or_else(|| {
        l.is(q, lead)
            .then(|| {
                l.run(q + 1, |c| {
                    !matches!(c, b':' | b'/' | b'<' | b'>') && !is_space(c)
                })
            })
            .filter(|&e| l.at(e) == Some(b':'))
            .map(|e| (e, false))
    });
    if let Some((c, valid)) = ns
        && let Some((e, lvalid)) = local(c + 1)
    {
        m.e = e as u32;
        m = m
            .g(first + usize::from(!valid), q, c)
            .g(first + 2, c, c + 1);
        return Some(m.g(first + 3 + usize::from(!lvalid), c + 1, e));
    }
    let (e, lvalid) = local(q)?;
    m.e = e as u32;
    Some(m.g(first + 3 + usize::from(!lvalid), q, e))
}

/// [`attribute`] at `p`.
pub(crate) fn attribute_at(l: &Line, p: usize) -> Option<M> {
    let free = |c: u8| !matches!(c, b'=' | b'/' | b'<' | b'>') && !is_space(c);
    let ahead = |e: usize| {
        l.is(e, |c| matches!(c, b'=' | b'<' | b'>') || is_space(c))
            || (l.is(e, |c| c == b'/' || c == b'?') && l.at(e + 1) == Some(b'>'))
    };
    // the local name at `at`: a name the look-ahead follows, else the shortest run it does
    let local = |at: usize| -> Option<(usize, bool)> {
        if l.is(at, is_name_start) {
            let e = l.run(at + 1, is_name);
            if ahead(e) {
                return Some((e, true));
            }
        }
        if !l.is(at, free) {
            return None;
        }
        let mut e = at + 1;
        loop {
            if ahead(e) {
                return Some((e, false));
            }
            if !l.is(e, free) {
                return None;
            }
            e += 1;
        }
    };
    if !l.is(p, free) {
        return None;
    }
    let ns = if l.is(p, is_name_start) {
        let e = l.run(p + 1, is_name);
        (l.at(e) == Some(b':')).then_some((e, true))
    } else {
        None
    };
    let ns = ns.or_else(|| {
        let e = l.run(p, |c| c != b':' && free(c));
        (e > p && l.at(e) == Some(b':')).then_some((e, false))
    });
    if let Some((c, valid)) = ns
        && let Some((e, lvalid)) = local(c + 1)
    {
        let m = M::new(p, e).g(1 + usize::from(!valid), p, c).g(3, c, c + 1);
        return Some(m.g(4 + usize::from(!lvalid), c + 1, e));
    }
    let (e, lvalid) = local(p)?;
    Some(M::new(p, e).g(4 + usize::from(!lvalid), p, e))
}
