//! XML, in place of bat's `XML`: its 68 contexts on the [machine](crate::machine), context for
//! context, each regex a matcher below. Tags, comments, CDATA sections and DTDs run on across
//! lines.
//!
//! The contexts keep syntect's ids (the order the probe's `dump XML` lists them in). Names'
//! `\p{L}` and `\p{N}` are ASCII letters and digits here, and any byte past ASCII, so that a
//! name never ends inside a character; on ASCII text the scopes are syntect's.

use crate::machine::{
    Ctx, Grammar, It::I, It::P, Kinds, Line, M, Pat, atoms, empty, first, is_space, is_word, lit1,
    non_space, protos, slots,
};

pub(crate) const CDATA_0: u8 = 0;
pub(crate) const COMMENT_0: u8 = 1;
pub(crate) const ATTLIST_PARENS_0: u8 = 2;
pub(crate) const ELEMENT_PARENS_0: u8 = 3;
pub(crate) const SUBSET_BRACKETS_0: u8 = 4;
pub(crate) const UNKNOWN_0: u8 = 5;
pub(crate) const PREPROCESSOR_0: u8 = 6;
pub(crate) const PREPROCESSOR_1: u8 = 7;
pub(crate) const KEY_VALUE_0: u8 = 8;
pub(crate) const TAG_0: u8 = 9;
pub(crate) const TAG_1: u8 = 10;
pub(crate) const MAIN_0: u8 = 11;
pub(crate) const START: u8 = 12;
pub(crate) const CDATA: u8 = 13;
pub(crate) const COMMENT: u8 = 14;
pub(crate) const DOCTYPE: u8 = 15;
pub(crate) const DOCTYPE_META: u8 = 16;
pub(crate) const DOCTYPE_ROOT_NAME: u8 = 17;
pub(crate) const DTD: u8 = 18;
pub(crate) const DTD_ATTLIST: u8 = 19;
pub(crate) const DTD_ATTLIST_CONTENT: u8 = 20;
pub(crate) const DTD_ATTLIST_META: u8 = 21;
pub(crate) const DTD_ATTLIST_PARENS: u8 = 22;
pub(crate) const DTD_COMMON_NAME: u8 = 23;
pub(crate) const DTD_CONTENT_QUOTED: u8 = 24;
pub(crate) const DTD_CONTENT_TYPE: u8 = 25;
pub(crate) const DTD_CONTENT_UNQUOTED: u8 = 26;
pub(crate) const DTD_ELEMENT: u8 = 27;
pub(crate) const DTD_ELEMENT_CONTENT: u8 = 28;
pub(crate) const DTD_ELEMENT_META: u8 = 29;
pub(crate) const DTD_ELEMENT_NAME: u8 = 30;
pub(crate) const DTD_ELEMENT_PARENS: u8 = 31;
pub(crate) const DTD_ELSE_POP: u8 = 32;
pub(crate) const DTD_END: u8 = 33;
pub(crate) const DTD_ENTITY: u8 = 34;
pub(crate) const DTD_ENTITY_CONTENT: u8 = 35;
pub(crate) const DTD_ENTITY_META: u8 = 36;
pub(crate) const DTD_ENTITY_NAME: u8 = 37;
pub(crate) const DTD_ENTITY_PUNCTUATION: u8 = 38;
pub(crate) const DTD_NOTATION: u8 = 39;
pub(crate) const DTD_NOTATION_META: u8 = 40;
pub(crate) const DTD_NOTATION_NAME: u8 = 41;
pub(crate) const DTD_SUBSET: u8 = 42;
pub(crate) const DTD_SUBSET_BRACKETS: u8 = 43;
pub(crate) const DTD_SUBSET_META: u8 = 44;
pub(crate) const DTD_SUBSET_NAME: u8 = 45;
pub(crate) const DTD_UNKNOWN: u8 = 46;
pub(crate) const ENTITY: u8 = 47;
pub(crate) const ENTITY_PARAMETER: u8 = 48;
pub(crate) const MAIN: u8 = 49;
pub(crate) const PREPROCESSOR: u8 = 50;
pub(crate) const PREPROCESSOR_END: u8 = 51;
pub(crate) const SHOULD_BE_ENTITY: u8 = 52;
pub(crate) const STRING_DOUBLE_QUOTED: u8 = 53;
pub(crate) const STRING_DOUBLE_QUOTED_BODY: u8 = 54;
pub(crate) const STRING_DOUBLE_QUOTED_POP: u8 = 55;
pub(crate) const STRING_QUOTED: u8 = 56;
pub(crate) const STRING_QUOTED_POP: u8 = 57;
pub(crate) const STRING_SINGLE_QUOTED: u8 = 58;
pub(crate) const STRING_SINGLE_QUOTED_BODY: u8 = 59;
pub(crate) const STRING_SINGLE_QUOTED_POP: u8 = 60;
pub(crate) const STRING_UNQUOTED: u8 = 61;
// nothing includes it, in bat's grammar either
#[allow(dead_code)]
pub(crate) const STRING_UNQUOTED_POP: u8 = 62;
pub(crate) const TAG: u8 = 63;
pub(crate) const TAG_ATTRIBUTE: u8 = 64;
pub(crate) const TAG_ATTRIBUTE_SEPARATOR: u8 = 65;
pub(crate) const TAG_ELSE_POP: u8 = 66;
pub(crate) const TAG_END_MISSING_POP: u8 = 67;

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

const fn ctx(items: &'static [crate::machine::It]) -> Ctx {
    Ctx::new(items)
}

const fn meta(meta: &'static [crate::machine::Atom], items: &'static [crate::machine::It]) -> Ctx {
    Ctx::new(items).meta(meta)
}

static CTXS: [Ctx; 68] = [
    // #anon_cdata_0
    Ctx::new(&[P(Pat::new(cdata_end).sc(&[s::TAG_END]).pop())])
        .meta(&[s::CDATA_META])
        .content(&[s::CDATA_TEXT]),
    // #anon_comment_0
    meta(
        &[s::COMMENT_BLOCK],
        &[
            P(Pat::new(comment_end).sc(&[s::COMMENT_END]).pop()),
            P(Pat::new(dashes).sc(&[s::DOUBLE_HYPHEN])),
        ],
    ),
    // #anon_dtd-attlist-parens_0
    meta(
        &[s::GROUP_ENUMERATED],
        &[
            P(Pat::new(close_paren).sc(&[s::GROUP_END]).pop()),
            P(Pat::new(pipe).sc(&[s::LOGICAL_SEP])),
            I(ENTITY_PARAMETER),
            I(ENTITY),
            I(SHOULD_BE_ENTITY),
            I(STRING_UNQUOTED),
        ],
    ),
    // #anon_dtd-element-parens_0
    ctx(&[
        P(Pat::new(quantifier).sc(&[s::OPERATOR]).pop()),
        I(DTD_ELSE_POP),
    ]),
    // #anon_dtd-subset-brackets_0
    meta(
        &[s::BRACKETS, s::INTERNAL_SUBSET],
        &[
            P(Pat::new(close_bracket).sc(&[s::BRACKETS_END]).pop()),
            I(DTD),
        ],
    ),
    // #anon_dtd-unknown_0
    meta(&[s::UNKNOWN_META], &[I(DTD_END)]),
    // #anon_preprocessor_0
    meta(
        &[s::PREPROCESSOR_META],
        &[
            I(PREPROCESSOR_END),
            I(TAG_END_MISSING_POP),
            I(TAG_ATTRIBUTE),
        ],
    ),
    // #anon_preprocessor_1
    meta(&[s::PREPROCESSOR_META], &[I(PREPROCESSOR_END)]),
    // #anon_tag-attribute-separator-key-value_0
    ctx(&[
        I(STRING_QUOTED_POP),
        P(Pat::new(bad_value).sc(&[s::BAD_ATTR_VALUE]).pop()),
        I(TAG_ELSE_POP),
    ]),
    // #anon_tag_0
    meta(
        &[s::TAG_META],
        &[
            P(Pat::new(gt).sc(&[s::TAG_END]).pop()),
            I(TAG_END_MISSING_POP),
            P(Pat::new(bad_tag_end).sc(&[s::BAD_TAG_END]).pop()),
            P(Pat::new(non_space).sc(&[s::UNEXPECTED_ATTR])),
        ],
    ),
    // #anon_tag_1
    meta(
        &[s::TAG_META],
        &[
            P(Pat::new(slash_gt).sc(&[s::TAG_END]).pop()),
            P(Pat::new(question_gt).sc(&[s::BAD_TAG_END]).pop()),
            I(TAG_END_MISSING_POP),
            I(TAG_ATTRIBUTE),
        ],
    ),
    // __main
    ctx(&[I(MAIN)]),
    // __start: its `text.xml` is the base scope
    ctx(&[P(Pat::new(empty).push(&[MAIN_0]))]),
    // cdata
    ctx(&[P(Pat::new(cdata_open)
        .caps(&[
            (1, &[s::TAG_BEGIN]),
            (2, &[s::CDATA_KW]),
            (3, &[s::TAG_BEGIN]),
        ])
        .push(&[CDATA_0]))]),
    // comment
    ctx(&[P(Pat::new(comment_open)
        .sc(&[s::COMMENT_BEGIN])
        .push(&[COMMENT_0]))]),
    // doctype
    ctx(&[P(Pat::new(doctype)
        .caps(&[
            (1, &[s::TAG_BEGIN]),
            (2, &[s::DOCTYPE_KW]),
            (3, &[s::BAD_TAG_NAME]),
        ])
        .push(&[
            DOCTYPE_META,
            DTD_SUBSET_BRACKETS,
            DTD_CONTENT_QUOTED,
            DTD_CONTENT_TYPE,
            DOCTYPE_ROOT_NAME,
        ]))]),
    // doctype-meta
    meta(&[s::DOCTYPE_META_S], &[I(DTD_END)]),
    // doctype-root-name
    ctx(&[
        P(Pat::new(root_name)
            .caps(&[
                (1, &[s::ROOT_NS]),
                (2, &[s::BAD_TAG_NAME]),
                (3, &[s::ROOT, s::NS_SEP]),
                (4, &[s::ROOT_LOCAL]),
                (5, &[s::BAD_TAG_NAME]),
            ])
            .pop()),
        I(DTD_ELSE_POP),
    ]),
    // dtd
    ctx(&[
        I(CDATA),
        I(COMMENT),
        I(DTD_ENTITY),
        I(DTD_ELEMENT),
        I(DTD_ATTLIST),
        I(DTD_NOTATION),
        I(DTD_SUBSET),
        I(DTD_UNKNOWN),
        I(ENTITY_PARAMETER),
        I(PREPROCESSOR),
    ]),
    // dtd-attlist
    ctx(&[P(Pat::new(attlist_open)
        .caps(&[(1, &[s::TAG_BEGIN]), (2, &[s::ATTLIST_KW])])
        .push(&[
            DTD_ATTLIST_META,
            DTD_ATTLIST_CONTENT,
            DTD_ELEMENT_NAME,
        ]))]),
    // dtd-attlist-content
    ctx(&[
        I(ENTITY_PARAMETER),
        I(DTD_ATTLIST_PARENS),
        P(Pat::new(attr_type).sc(&[s::ATTR_TYPE])),
        P(Pat::new(attr_default)
            .sc(&[s::ATTR_DEFAULT])
            .caps(&[(1, &[s::STORAGE_MARK])])),
        P(Pat::new(dtd_attr_name).sc(&[s::ATTR_NAME])),
        I(DTD_CONTENT_QUOTED),
    ]),
    // dtd-attlist-meta
    meta(&[s::ATTLIST_META], &[I(DTD_END)]),
    // dtd-attlist-parens
    ctx(&[P(Pat::new(open_paren)
        .sc(&[s::GROUP_BEGIN])
        .push(&[ATTLIST_PARENS_0]))]),
    // dtd-common-name
    ctx(&[
        P(Pat::new(parameter)
            .sc(&[s::PARAMETER])
            .caps(&[(1, &[s::PARAMETER_MARK]), (2, &[s::PARAMETER_END])])
            .pop()),
        P(Pat::new(bad_identifier).sc(&[s::BAD_IDENTIFIER]).pop()),
        I(DTD_ELSE_POP),
    ]),
    // dtd-content-quoted
    ctx(&[I(STRING_QUOTED), I(DTD_ELSE_POP)]),
    // dtd-content-type
    ctx(&[
        P(Pat::new(public_system).sc(&[s::EXTERNAL]).pop()),
        I(DTD_ELSE_POP),
    ]),
    // dtd-content-unquoted
    ctx(&[I(STRING_UNQUOTED), I(DTD_ELSE_POP)]),
    // dtd-element
    ctx(&[P(Pat::new(element_open)
        .caps(&[(1, &[s::TAG_BEGIN]), (2, &[s::ELEMENT_KW])])
        .push(&[
            DTD_ELEMENT_META,
            DTD_ELEMENT_CONTENT,
            DTD_ELEMENT_NAME,
        ]))]),
    // dtd-element-content
    ctx(&[
        P(Pat::new(empty_any).sc(&[s::CONSTANT]).pop()),
        P(Pat::new(open_paren)
            .sc(&[s::GROUP_BEGIN])
            .set(&[DTD_ELEMENT_PARENS])),
        I(ENTITY_PARAMETER),
        I(DTD_CONTENT_QUOTED),
    ]),
    // dtd-element-meta
    meta(&[s::ELEMENT_META], &[I(DTD_END)]),
    // dtd-element-name
    ctx(&[
        P(Pat::new(dtd_name).sc(&[s::ELEMENT_NAME]).pop()),
        I(DTD_COMMON_NAME),
    ]),
    // dtd-element-parens
    meta(
        &[s::GROUP],
        &[
            P(Pat::new(close_paren)
                .sc(&[s::GROUP_END])
                .set(&[ELEMENT_PARENS_0])),
            P(Pat::new(open_paren)
                .sc(&[s::GROUP_BEGIN])
                .push(&[DTD_ELEMENT_PARENS])),
            P(Pat::new(quantifier).sc(&[s::OPERATOR])),
            P(Pat::new(comma_pipe).sc(&[s::SEPARATOR])),
            I(ENTITY_PARAMETER),
            I(ENTITY),
            I(SHOULD_BE_ENTITY),
            I(STRING_UNQUOTED),
        ],
    ),
    // dtd-else-pop
    ctx(&[P(Pat::new(bracket_ahead).pop()), I(TAG_ELSE_POP)]),
    // dtd-end
    ctx(&[
        P(Pat::new(gt).sc(&[s::TAG_END]).pop()),
        P(Pat::new(missing_dtd_end).sc(&[s::MISSING_TAG_END]).pop()),
        P(Pat::new(bad_tag_end).sc(&[s::BAD_TAG_END]).pop()),
        P(Pat::new(non_space).sc(&[s::UNEXPECTED])),
    ]),
    // dtd-entity
    ctx(&[P(Pat::new(entity_open)
        .caps(&[(1, &[s::TAG_BEGIN]), (2, &[s::ENTITY_KW])])
        .push(&[
            DTD_ENTITY_META,
            DTD_ENTITY_CONTENT,
            DTD_CONTENT_TYPE,
            DTD_ENTITY_NAME,
            DTD_ENTITY_PUNCTUATION,
        ]))]),
    // dtd-entity-content
    ctx(&[
        P(Pat::new(ndata).sc(&[s::NDATA]).set(&[DTD_CONTENT_UNQUOTED])),
        I(DTD_CONTENT_QUOTED),
    ]),
    // dtd-entity-meta
    meta(&[s::ENTITY_META], &[I(DTD_END)]),
    // dtd-entity-name
    ctx(&[
        P(Pat::new(dtd_name).sc(&[s::ENTITY_NAME]).pop()),
        I(DTD_COMMON_NAME),
    ]),
    // dtd-entity-punctuation
    ctx(&[
        P(Pat::new(percent).sc(&[s::ENTITY_MARK]).pop()),
        I(DTD_ELSE_POP),
    ]),
    // dtd-notation
    ctx(&[P(Pat::new(notation_open)
        .caps(&[(1, &[s::TAG_BEGIN]), (2, &[s::NOTATION_KW])])
        .push(&[
            DTD_NOTATION_META,
            DTD_CONTENT_QUOTED,
            DTD_CONTENT_TYPE,
            DTD_NOTATION_NAME,
        ]))]),
    // dtd-notation-meta
    meta(&[s::NOTATION_META], &[I(DTD_END)]),
    // dtd-notation-name
    ctx(&[
        P(Pat::new(dtd_name).sc(&[s::NOTATION_NAME]).pop()),
        I(DTD_COMMON_NAME),
    ]),
    // dtd-subset
    ctx(&[P(Pat::new(subset_open).sc(&[s::TAG_BEGIN]).push(&[
        DTD_SUBSET_META,
        DTD_SUBSET_BRACKETS,
        DTD_SUBSET_NAME,
    ]))]),
    // dtd-subset-brackets
    ctx(&[
        P(Pat::new(open_bracket)
            .sc(&[s::BRACKETS_BEGIN])
            .set(&[SUBSET_BRACKETS_0])),
        I(DTD_ELSE_POP),
    ]),
    // dtd-subset-meta
    meta(
        &[s::SUBSET_META],
        &[
            P(Pat::new(bracket_gt).sc(&[s::TAG_END]).pop()),
            P(Pat::new(maybe_bad_gt).sc(&[s::BAD_TAG_END]).pop()),
            I(TAG_END_MISSING_POP),
        ],
    ),
    // dtd-subset-name
    ctx(&[
        P(Pat::new(dtd_name).sc(&[s::SUBSET_NAME]).pop()),
        I(DTD_COMMON_NAME),
    ]),
    // dtd-unknown
    ctx(&[P(Pat::new(unknown_open)
        .caps(&[(1, &[s::TAG_BEGIN]), (2, &[s::BAD_TAG_NAME])])
        .push(&[UNKNOWN_0]))]),
    // entity
    ctx(&[
        P(Pat::new(entity_hex)
            .sc(&[s::ENTITY_HEX])
            .caps(&[(1, &[s::ENTITY_MARK]), (2, &[s::ENTITY_END])])),
        P(Pat::new(entity_dec)
            .sc(&[s::ENTITY_DEC])
            .caps(&[(1, &[s::ENTITY_MARK]), (2, &[s::ENTITY_END])])),
        P(Pat::new(entity_named)
            .sc(&[s::ENTITY_NAMED])
            .caps(&[(1, &[s::ENTITY_MARK]), (2, &[s::ENTITY_END])])),
    ]),
    // entity-parameter
    ctx(&[
        P(Pat::new(placeholder)
            .sc(&[s::PLACEHOLDER])
            .caps(&[(1, &[s::CONSTANT_MARK])])),
        P(Pat::new(parameter)
            .sc(&[s::PARAMETER])
            .caps(&[(1, &[s::PARAMETER_MARK]), (2, &[s::PARAMETER_END])])),
    ]),
    // main: only ever included, so its `text.xml` never applies
    ctx(&[
        I(PREPROCESSOR),
        I(DOCTYPE),
        I(COMMENT),
        I(CDATA),
        I(TAG),
        I(ENTITY),
        I(SHOULD_BE_ENTITY),
    ]),
    // preprocessor
    ctx(&[
        P(Pat::new(preprocessor_xml)
            .caps(&[
                (1, &[s::TAG_BEGIN]),
                (2, &[s::TAG_NAME]),
                (3, &[s::BAD_TAG_NAME]),
            ])
            .push(&[PREPROCESSOR_0])),
        P(Pat::new(preprocessor_other)
            .caps(&[(1, &[s::TAG_BEGIN]), (2, &[s::TAG_NAME])])
            .push(&[PREPROCESSOR_1])),
    ]),
    // preprocessor-end
    ctx(&[P(Pat::new(question_gt).sc(&[s::TAG_END]).pop())]),
    // should-be-entity
    ctx(&[
        P(Pat::new(ampersand).sc(&[s::BAD_AMPERSAND])),
        P(Pat::new(lt).sc(&[s::MISSING_ENTITY])),
    ]),
    // string-double-quoted
    ctx(&[P(Pat::new(dquote)
        .sc(&[s::STR_BEGIN])
        .push(&[STRING_DOUBLE_QUOTED_BODY]))]),
    // string-double-quoted-body
    meta(
        &[s::DOUBLE],
        &[
            P(Pat::new(dquote).sc(&[s::STR_END]).pop()),
            I(TAG_END_MISSING_POP),
            I(ENTITY),
            I(SHOULD_BE_ENTITY),
        ],
    ),
    // string-double-quoted-pop
    ctx(&[P(Pat::new(dquote)
        .sc(&[s::STR_BEGIN])
        .set(&[STRING_DOUBLE_QUOTED_BODY]))]),
    // string-quoted
    ctx(&[I(STRING_DOUBLE_QUOTED), I(STRING_SINGLE_QUOTED)]),
    // string-quoted-pop
    ctx(&[I(STRING_DOUBLE_QUOTED_POP), I(STRING_SINGLE_QUOTED_POP)]),
    // string-single-quoted
    ctx(&[P(Pat::new(squote)
        .sc(&[s::STR_BEGIN])
        .push(&[STRING_SINGLE_QUOTED_BODY]))]),
    // string-single-quoted-body
    meta(
        &[s::SINGLE],
        &[
            P(Pat::new(squote).sc(&[s::STR_END]).pop()),
            I(TAG_END_MISSING_POP),
            I(ENTITY),
            I(SHOULD_BE_ENTITY),
        ],
    ),
    // string-single-quoted-pop
    ctx(&[P(Pat::new(squote)
        .sc(&[s::STR_BEGIN])
        .set(&[STRING_SINGLE_QUOTED_BODY]))]),
    // string-unquoted
    ctx(&[P(Pat::new(unquoted).sc(&[s::UNQUOTED]))]),
    // string-unquoted-pop
    ctx(&[P(Pat::new(unquoted).sc(&[s::UNQUOTED]).pop())]),
    // tag
    ctx(&[
        P(Pat::new(close_tag)
            .caps(&[
                (1, &[s::TAG_BEGIN]),
                (2, &[s::TAG_NS]),
                (3, &[s::BAD_TAG_NAME]),
                (4, &[s::TAG_NAME, s::NS_SEP]),
                (5, &[s::TAG_LOCAL]),
                (6, &[s::BAD_TAG_NAME]),
            ])
            .push(&[TAG_0])),
        P(Pat::new(open_tag)
            .caps(&[
                (1, &[s::TAG_BEGIN]),
                (2, &[s::TAG_NS]),
                (3, &[s::BAD_TAG_NAME]),
                (4, &[s::TAG_NAME, s::NS_SEP]),
                (5, &[s::TAG_LOCAL]),
                (6, &[s::BAD_TAG_NAME]),
            ])
            .push(&[TAG_1])),
    ]),
    // tag-attribute
    ctx(&[P(Pat::new(attribute)
        .caps(&[
            (1, &[s::ATTR_NS]),
            (2, &[s::BAD_ATTR_NAME]),
            (3, &[s::ATTR_NAME, s::NS_SEP]),
            (4, &[s::ATTR_LOCAL]),
            (5, &[s::BAD_ATTR_NAME]),
        ])
        .push(&[TAG_ATTRIBUTE_SEPARATOR]))]),
    // tag-attribute-separator-key-value
    ctx(&[
        P(Pat::new(equals).sc(&[s::KEY_VALUE]).set(&[KEY_VALUE_0])),
        I(TAG_ELSE_POP),
    ]),
    // tag-else-pop
    ctx(&[P(Pat::new(tag_else_ahead).pop())]),
    // tag-end-missing-pop
    ctx(&[P(Pat::new(missing_tag_end).sc(&[s::MISSING_TAG_END]).pop())]),
];

static SLOTS: [u16; 68] = slots(&CTXS);
static PROTOS: [bool; 68] = protos(&CTXS, None);
static KINDS: Kinds = Kinds::new();

pub static GRAMMAR: Grammar = Grammar {
    ctxs: &CTXS,
    start: &[START],
    atoms: ATOMS,
    slots: &SLOTS,
    proto: None,
    protos: &PROTOS,
    kinds: &KINDS,
};

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

/// Whether `l`'s byte at `i` is a space (`\s`), the `\n` among them.
fn space_at(l: &Line, i: usize) -> bool {
    l.is(i, is_space)
}

/// `\b` at `i`.
pub(crate) fn boundary(l: &Line, i: usize) -> bool {
    let w = |i: usize| l.is(i, |c| is_word(c) && c != b'\n');
    (i > 0 && w(i - 1)) != w(i)
}

/// The leftmost `lit` that `\b` follows, at or after `from`.
fn lit_word(l: &Line, from: usize, lit: &[u8]) -> Option<usize> {
    let mut at = from;
    while let Some(m) = l.lit(at, lit) {
        if boundary(l, m.e as usize) {
            return Some(m.s as usize);
        }
        at = m.s as usize + 1;
    }
    None
}

/// `]]>`
fn cdata_end(l: &Line, from: usize) -> Option<M> {
    l.lit(from, b"]]>")
}

/// `-->`
fn comment_end(l: &Line, from: usize) -> Option<M> {
    l.lit(from, b"-->")
}

/// `-{2,}`
fn dashes(l: &Line, from: usize) -> Option<M> {
    let k = l.lit(from, b"--")?.s as usize;
    Some(M::new(k, l.run(k, |c| c == b'-')))
}

/// `\)`
fn close_paren(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b')')
}

/// `\(`
fn open_paren(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'(')
}

/// `\|`
fn pipe(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'|')
}

/// The leftmost byte in `set`, as a match.
fn one_of(l: &Line, from: usize, set: &[u8]) -> Option<M> {
    let hay = l.b.get(from..)?;
    let i = from + hay.iter().position(|c| set.contains(c))?;
    Some(M::new(i, i + 1))
}

/// `[*?+]`
fn quantifier(l: &Line, from: usize) -> Option<M> {
    one_of(l, from, b"*?+")
}

/// `[,|]`
fn comma_pipe(l: &Line, from: usize) -> Option<M> {
    one_of(l, from, b",|")
}

/// `\]`
fn close_bracket(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b']')
}

/// `\[`
fn open_bracket(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'[')
}

/// `>`
fn gt(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'>')
}

/// `%`
fn percent(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'%')
}

/// `=`
fn equals(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'=')
}

/// `"`
fn dquote(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'"')
}

/// `'`
fn squote(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'\'')
}

/// `&`
fn ampersand(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'&')
}

/// `<`
fn lt(l: &Line, from: usize) -> Option<M> {
    lit1(l, from, b'<')
}

/// `<!--`
fn comment_open(l: &Line, from: usize) -> Option<M> {
    l.lit(from, b"<!--")
}

/// `<!\[`
fn subset_open(l: &Line, from: usize) -> Option<M> {
    l.lit(from, b"<![")
}

/// `\]>`
fn bracket_gt(l: &Line, from: usize) -> Option<M> {
    l.lit(from, b"]>")
}

/// `\?>`
pub(crate) fn question_gt(l: &Line, from: usize) -> Option<M> {
    l.lit(from, b"?>")
}

/// `>` with the byte before it when that is one of `before`, as `[…]?>` takes it; or, with
/// `must`, only a `>` that one of them comes before (`[…]>`).
fn before_gt(l: &Line, from: usize, before: &[u8], must: bool) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'>') {
        if k > from && before.contains(&l.b[k - 1]) {
            return Some(M::new(k - 1, k + 1));
        }
        if !must {
            return Some(M::new(k, k + 1));
        }
        at = k + 1;
    }
    None
}

/// `[/\?]>`
fn bad_tag_end(l: &Line, from: usize) -> Option<M> {
    before_gt(l, from, b"/?", true)
}

/// `[/\?]?>`
fn maybe_bad_gt(l: &Line, from: usize) -> Option<M> {
    before_gt(l, from, b"/?", false)
}

/// `/?>`
pub(crate) fn slash_gt(l: &Line, from: usize) -> Option<M> {
    before_gt(l, from, b"/", false)
}

/// `[^?/<>\s]+`
fn bad_value(l: &Line, from: usize) -> Option<M> {
    let ok = |c: u8| !matches!(c, b'?' | b'/' | b'<' | b'>') && !is_space(c);
    let hay = l.b.get(from..)?;
    let i = from + hay.iter().position(|&c| ok(c))?;
    Some(M::new(i, l.run(i, ok)))
}

/// `(<!\[)(CDATA)(\[)`
fn cdata_open(l: &Line, from: usize) -> Option<M> {
    let k = l.lit(from, b"<![CDATA[")?.s as usize;
    Some(
        M::new(k, k + 9)
            .g(1, k, k + 3)
            .g(2, k + 3, k + 8)
            .g(3, k + 8, k + 9),
    )
}

/// `(<!)(?:(DOCTYPE)|(?i:(DOCTYPE)))\b`
fn doctype(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(m) = l.lit(at, b"<!") {
        let k = m.s as usize;
        if let Some(m) = doctype_at(l, k) {
            return Some(m);
        }
        at = k + 1;
    }
    None
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

/// `(<!)(KW)\b`
fn declaration(l: &Line, from: usize, kw: &[u8]) -> Option<M> {
    let mut at = from;
    while let Some(m) = l.lit(at, b"<!") {
        let k = m.s as usize;
        if let Some(m) = declaration_at(l, k, kw) {
            return Some(m);
        }
        at = k + 1;
    }
    None
}

/// [`declaration`] at `k`, where a `<!` is.
pub(crate) fn declaration_at(l: &Line, k: usize, kw: &[u8]) -> Option<M> {
    let e = k + 2 + kw.len();
    (l.b.get(k + 2..e) == Some(kw) && boundary(l, e))
        .then(|| M::new(k, e).g(1, k, k + 2).g(2, k + 2, e))
}

/// `(<!)(ATTLIST)\b`
fn attlist_open(l: &Line, from: usize) -> Option<M> {
    declaration(l, from, b"ATTLIST")
}

/// `(<!)(ELEMENT)\b`
fn element_open(l: &Line, from: usize) -> Option<M> {
    declaration(l, from, b"ELEMENT")
}

/// `(<!)(ENTITY)\b`
fn entity_open(l: &Line, from: usize) -> Option<M> {
    declaration(l, from, b"ENTITY")
}

/// `(<!)(NOTATION)\b`
fn notation_open(l: &Line, from: usize) -> Option<M> {
    declaration(l, from, b"NOTATION")
}

/// `(<!)([^?/<>\s]*)`
fn unknown_open(l: &Line, from: usize) -> Option<M> {
    Some(unknown_at(l, l.lit(from, b"<!")?.s as usize))
}

/// [`unknown_open`] at `k`, where a `<!` is.
pub(crate) fn unknown_at(l: &Line, k: usize) -> M {
    let e = l.run(k + 2, |c| {
        !matches!(c, b'?' | b'/' | b'<' | b'>') && !is_space(c)
    });
    M::new(k, e).g(1, k, k + 2).g(2, k + 2, e)
}

/// `\b(?:CDATA|ENTITY|ENTITIES|IDREFS?|ID|NMTOKENS?|NOTATION)\b`
fn attr_type(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| attr_type_at(l, p))
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

/// `(#)(?:FIXED|IMPLIED|REQUIRED)\b`
fn attr_default(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'#') {
        if let Some(m) = attr_default_at(l, k) {
            return Some(m);
        }
        at = k + 1;
    }
    None
}

/// [`attr_default`] at `k`, where a `#` is.
pub(crate) fn attr_default_at(l: &Line, k: usize) -> Option<M> {
    let rest = &l.b[k + 1..];
    [&b"FIXED"[..], b"IMPLIED", b"REQUIRED"]
        .into_iter()
        .find(|w| rest.starts_with(w) && boundary(l, k + 1 + w.len()))
        .map(|w| M::new(k, k + 1 + w.len()).g(1, k, k + 1))
}

/// `\b[\p{L}:_][\p{L}\p{N}:_.-]*\b`: the run gives back characters until a `\b` follows.
fn dtd_attr_name(l: &Line, from: usize) -> Option<M> {
    let mut p = from;
    while p < l.nl() {
        if !(l.is(p, is_name2_start) && boundary(l, p)) {
            p += 1;
            continue;
        }
        if let Some(m) = dtd_attr_name_at(l, p) {
            return Some(m);
        }
        // no `\b` in the run: none from any start in it either
        p = l.run(p + 1, is_name2);
    }
    None
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

/// `(#)P?CDATA`
fn placeholder(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'#') {
        if let Some(m) = placeholder_at(l, k) {
            return Some(m);
        }
        at = k + 1;
    }
    None
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

/// `(<mark>)[\p{L}:_][\p{L}\p{N}:_.-]*(;)`
fn marked_name(l: &Line, from: usize, mark: u8) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, mark) {
        if let Some(m) = marked_name_at(l, k) {
            return Some(m);
        }
        at = k + 1;
    }
    None
}

/// [`marked_name`] at `k`, where the mark is.
pub(crate) fn marked_name_at(l: &Line, k: usize) -> Option<M> {
    if !l.is(k + 1, is_name2_start) {
        return None;
    }
    let e = l.run(k + 2, is_name2);
    (l.at(e) == Some(b';')).then(|| M::new(k, e + 1).g(1, k, k + 1).g(2, e, e + 1))
}

/// `(%)[\p{L}:_][\p{L}\p{N}:_.-]*(;)`
fn parameter(l: &Line, from: usize) -> Option<M> {
    marked_name(l, from, b'%')
}

/// `(&)[\p{L}:_][\p{L}\p{N}:_.-]*(;)`
fn entity_named(l: &Line, from: usize) -> Option<M> {
    marked_name(l, from, b'&')
}

/// `(&#[xX])\h+(;)`, or with `hex` false `(&#)[0-9]+(;)`.
fn char_ref(l: &Line, from: usize, hex: bool) -> Option<M> {
    let mut at = from;
    while let Some(m) = l.lit(at, b"&#") {
        let k = m.s as usize;
        if let Some(m) = char_ref_at(l, k, hex) {
            return Some(m);
        }
        at = k + 1;
    }
    None
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

fn entity_hex(l: &Line, from: usize) -> Option<M> {
    char_ref(l, from, true)
}

fn entity_dec(l: &Line, from: usize) -> Option<M> {
    char_ref(l, from, false)
}

/// `[^['"\[\]()<>\s]]+`
fn bad_identifier(l: &Line, from: usize) -> Option<M> {
    let ok = |c: u8| !b"'\"[]()<>".contains(&c) && !is_space(c);
    let hay = l.b.get(from..)?;
    let i = from + hay.iter().position(|&c| ok(c))?;
    Some(M::new(i, l.run(i, ok)))
}

/// The leftmost of `words` that `\b` follows; with `before`, that `\b` comes before too.
fn word(l: &Line, from: usize, words: &[&[u8]], before: bool) -> Option<M> {
    words
        .iter()
        .filter_map(|w| {
            let mut at = from;
            loop {
                let k = lit_word(l, at, w)?;
                if !before || boundary(l, k) {
                    return Some(M::new(k, k + w.len()));
                }
                at = k + 1;
            }
        })
        .min_by_key(|m| m.s)
}

/// `(?:PUBLIC|SYSTEM)\b`
fn public_system(l: &Line, from: usize) -> Option<M> {
    word(l, from, &[b"PUBLIC", b"SYSTEM"], false)
}

/// `\b(?:EMPTY|ANY)\b`
fn empty_any(l: &Line, from: usize) -> Option<M> {
    word(l, from, &[b"EMPTY", b"ANY"], true)
}

/// `NDATA\b`
fn ndata(l: &Line, from: usize) -> Option<M> {
    word(l, from, &[b"NDATA"], false)
}

/// `[\p{L}:_][\p{L}\p{N}:_.-]*(?=[['"\[\]()<>\s]])`
fn dtd_name(l: &Line, from: usize) -> Option<M> {
    let mut p = from;
    while p < l.nl() {
        if !l.is(p, is_name2_start) {
            p += 1;
            continue;
        }
        match dtd_name_at(l, p) {
            Some(m) => return Some(m),
            // every start in the run ends where it does
            None => p = l.run(p + 1, is_name2),
        }
    }
    None
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

/// `[\p{L}:_][\p{L}\p{N}:_.-]*`
fn unquoted(l: &Line, from: usize) -> Option<M> {
    let hay = l.b.get(from..)?;
    let i = from + hay.iter().position(|&c| is_name2_start(c))?;
    Some(M::new(i, l.run(i + 1, is_name2)))
}

/// `(?=\s?\])`
fn bracket_ahead(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| {
        let hit = l.at(p) == Some(b']') || (space_at(l, p) && l.at(p + 1) == Some(b']'));
        hit.then(|| M::new(p, p))
    })
}

/// `\s?(?=[<…])`: a space and the byte ahead, else the byte ahead alone.
fn space_then(l: &Line, from: usize, set: &[u8]) -> Option<M> {
    l.find(from, |p| {
        let is = |i: usize| l.at(i).is_some_and(|c| set.contains(&c));
        if space_at(l, p) && is(p + 1) {
            return Some(M::new(p, p + 1));
        }
        is(p).then(|| M::new(p, p))
    })
}

/// `\s?(?=[<\]])`
fn missing_dtd_end(l: &Line, from: usize) -> Option<M> {
    space_then(l, from, b"<]")
}

/// `\s?(?=<)`
pub(crate) fn missing_tag_end(l: &Line, from: usize) -> Option<M> {
    space_then(l, from, b"<")
}

/// `(?=\s?<|\S)`
fn tag_else_ahead(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| {
        let hit = (space_at(l, p) && l.at(p + 1) == Some(b'<'))
            || l.at(p) == Some(b'<')
            || (p < l.nl() && !is_space(l.b[p]));
        hit.then(|| M::new(p, p))
    })
}

/// `(<\?)(?:(xml(?:-[_a-z][-_a-z0-9]*)?)(?=[?<>\s])|([xX][mM][lL][^?<>\s]*))`
fn preprocessor_xml(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(m) = l.lit(at, b"<?") {
        let k = m.s as usize;
        if let Some(m) = preprocessor_xml_at(l, k) {
            return Some(m);
        }
        at = k + 1;
    }
    None
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

/// `(<\?)([\p{L}:_][\p{L}\p{N}:_.-]*)\b`
fn preprocessor_other(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(m) = l.lit(at, b"<?") {
        let k = m.s as usize;
        if let Some(m) = preprocessor_other_at(l, k) {
            return Some(m);
        }
        at = k + 1;
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

/// `(</)` and a qualified name
fn close_tag(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(m) = l.lit(at, b"</") {
        let k = m.s as usize;
        if let Some(m) = qname(l, k + 2, M::new(k, k).g(1, k, k + 2), 2) {
            return Some(m);
        }
        at = k + 1;
    }
    None
}

/// `(<)` and a qualified name
fn open_tag(l: &Line, from: usize) -> Option<M> {
    let mut at = from;
    while let Some(k) = first(l, at, b'<') {
        if let Some(m) = qname(l, k + 1, M::new(k, k).g(1, k, k + 1), 2) {
            return Some(m);
        }
        at = k + 1;
    }
    None
}

/// The document root's qualified name.
fn root_name(l: &Line, from: usize) -> Option<M> {
    l.find(from, |p| qname(l, p, M::new(p, p), 1))
}

/// An attribute's name:
/// `(?:(?:(N)|([^:=/<>\s]+))(:))?(?:(N)|([^=/<>\s]+?))(?=[=<>\s]|[/?]>)`.
pub(crate) fn attribute(l: &Line, from: usize) -> Option<M> {
    let free = |c: u8| !matches!(c, b'=' | b'/' | b'<' | b'>') && !is_space(c);
    let mut p = from;
    while p < l.nl() {
        if !l.is(p, free) {
            p += 1;
            continue;
        }
        if let Some(m) = attribute_at(l, p) {
            return Some(m);
        }
        // the run's end decides for every start in it
        p = l.run(p, free);
    }
    None
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
