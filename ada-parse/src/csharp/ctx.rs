//! bat's C# contexts, each an arm of [`Scan::context`]: its patterns in the grammar's order
//! at the scan's place (after the prototype, which [`Scan::at`] tries first). The pattern
//! groups the grammar includes in many contexts (`main`, `line_of_code`,
//! `line_of_code_in_no_semicolon`, `keywords`, `literals`, `type`, `method_declaration`) are
//! methods of their own.

use super::c::*;
use super::*;

/// `{{bin_op}}`'s alternatives, in order.
const BIN_OP: [&[u8]; 21] = [
    b"+", b"->", b"-", b"*", b"/", b"%", b"||", b"&&", b"|", b"&", b"^", b"<<", b">>", b"=>",
    b"<=", b"<", b">=", b">", b"==", b"!=", b"??",
];
/// `{{unary_op}}`'s alternatives, in order.
const UNARY_OP: [&[u8]; 7] = [b"++", b"--", b"-", b"~", b"!", b"&", b"*"];

/// The words `line_of_code_in_no_semicolon` takes for LINQ.
const SQL_WORDS: [&[u8]; 15] = [
    b"let",
    b"select",
    b"equals",
    b"from",
    b"in",
    b"where",
    b"join",
    b"orderby",
    b"group",
    b"by",
    b"on",
    b"into",
    b"into",
    b"ascending",
    b"descending",
];

impl Scan<'_, '_> {
    /// The patterns of context `top` at `p`, the first that matches applied.
    pub(super) fn context(&mut self, top: u8, p: usize) -> Option<usize> {
        match top {
            MAIN => self.main(p),
            MAIN_BRACE => self.brace_main(p, BLOCK_END),
            MAIN_BRACE_M => self.brace_main(p, M_BLOCK_END),
            MAIN_LINE => {
                if self.main_line_stop(p)
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.line(p)
            }
            LINE => self.line(p),
            LINE_IN => self.line_in(p),
            NOSEMI => self.nosemi(p),
            ENUM_HEAD => {
                (self.ch(p) == b'{').then_some(())?;
                self.then(p, p + 1, BLOCK_BEGIN, &[ENUM_BODY])
            }
            ENUM_BODY => self.enum_body(p),
            ENUM_END => {
                if self.ch(p) == b';' {
                    return self.close(p, p + 1, TERM_STMT);
                }
                self.any_leave(p)
            }
            USING_NS => self.using_ns(p),
            NS_DECL => self.ns_decl(p),
            NS_NAME => self.ns_name(p),
            DELEGATE_TYPE => {
                if self.space(p) {
                    let e = self.next(p);
                    return self.then(p, e, PLAIN, &[DELEGATE_NAME]);
                }
                self.type_(p)
            }
            DELEGATE_NAME => self.delegate_name(p),
            DN_GENERIC => {
                if self.ch(p) == b'>' {
                    return self.then(p, p + 1, DELEGATE_GENERIC_END, &[DN_AFTER]);
                }
                self.type_parameter(p)
            }
            DN_AFTER => {
                if let Some(e) = self.sp1(p) {
                    return self.tok(p, e, DELEGATE);
                }
                (self.ch(p) == b'(').then_some(())?;
                self.then(p, p + 1, DELEGATE_PARAMS_BEGIN, &[DELEGATE_PARAMS])
            }
            DELEGATE_PARAMS => {
                if self.ch(p) == b')' {
                    return self.then(p, p + 1, PARAMS_END, &[DELEGATE_END]);
                }
                self.any_enter(p, &[METHOD_PARAM, METHOD_PARAM_TYPE])
            }
            DELEGATE_END => self.delegate_end(p),
            DATA_TYPE_SIG => self.data_type_sig(p),
            DTS_GENERIC | MN_GENERIC => self.type_parameter(p),
            TYPE_CONSTRAINT => self.type_constraint(p),
            DATA_TYPE_BODY => self.data_type_body(p),
            DTB_BLOCK => self.dtb_block(p),
            EVENT_HANDLER => self.event_handler(p),
            EH_AFTER => match self.ch(p) {
                b';' if p < self.n => self.close(p, p + 1, TERM_STMT),
                b'{' => self.then(p, p + 1, BLOCK_BEGIN, &[EH_BLOCK]),
                _ => self.any_leave(p),
            },
            EH_BLOCK => {
                if self.ch(p) == b'}' {
                    return self.close(p, p + 1, BLOCK_END);
                }
                if self.kw(p, b"add") {
                    return self.tok_push(p, p + 3, ADD, &[METHOD_BODY]);
                }
                if self.kw(p, b"remove") {
                    return self.tok_push(p, p + 6, REMOVE, &[METHOD_BODY]);
                }
                None
            }
            METHOD_NAME => self.method_name(p),
            MN_EXPLICIT => self.mn_explicit(p),
            MN_AFTER => {
                let s = self.sp(p);
                if self.ch(s) == b',' && s < self.n {
                    self.put(s, s + 1, VARIABLES_SEP);
                    self.fill(s + 1);
                    self.pop();
                    return Some(s + 1);
                }
                self.leave(p)
            }
            MD_TYPE => {
                if p == self.step {
                    let x = self.type_suffix_capture(p);
                    if x.end > p {
                        self.suffix_tokens(&x, None);
                        return Some(x.end);
                    }
                }
                self.shift(p, &[METHOD_NAME])
            }
            METHOD_BODY => self.method_body(p),
            MB_BLOCK => {
                if self.ch(p) == b'}' {
                    return self.close(p, p + 1, BLOCK_END);
                }
                if let Some(q) = self.stray(p) {
                    return Some(q);
                }
                self.code_block_in(p)
            }
            METHOD_PARAMS => {
                if self.ch(p) == b')' {
                    return self.close(p, p + 1, PARAMS_END_M);
                }
                self.any_enter(p, &[METHOD_PARAM, METHOD_PARAM_TYPE])
            }
            METHOD_PARAMS_BRACKET => {
                if self.ch(p) == b']' {
                    return self.close(p, p + 1, M_BRACKETS_END);
                }
                self.any_enter(p, &[METHOD_PARAM, METHOD_PARAM_TYPE])
            }
            METHOD_PARAM => self.method_param(p),
            METHOD_PARAM_TYPE => {
                if let Some(q) = self.attribute(p) {
                    return Some(q);
                }
                let c = self.ch(p);
                if self.nonspace(p) && c != b'[' {
                    return self.shift(p, &[MPT_TYPE]);
                }
                if matches!(c, b'}' | b')' | b'>' | b']' | b';')
                    && p < self.n
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                None
            }
            MPT_TYPE => self.mpt_type(p),
            CTOR_PREBODY => {
                if self.ch(p) == b':' && p < self.n {
                    return self.then(p, p + 1, PREBODY_SEP, &[CTOR_COLON]);
                }
                if self.ch(p) == b'{' || self.lit(p, b"=>") {
                    return self.shift(p, &[METHOD_BODY]);
                }
                None
            }
            CTOR_COLON => self.ctor_colon(p),
            CTOR_INIT => {
                if self.ch(p) == b')' {
                    return self.tok(p, p + 1, GROUP_END);
                }
                self.shift(p, &[CTOR_INIT_AFTER])
            }
            CTOR_INIT_AFTER => {
                if self.ch(p) == b'{' {
                    return self.shift(p, &[METHOD_BODY]);
                }
                self.any_leave(p)
            }
            METHOD_ACCESSOR => match self.ch(p) {
                b'{' => self.then(p, p + 1, M_BLOCK_BEGIN, &[ACC_BLOCK]),
                b'=' if self.lit(p, b"=>") => self.then(p, p + 2, FUNCTION_ARROW, &[LINE_IN]),
                _ if self.nonspace(p) => {
                    let e = self.next(p);
                    self.close(p, e, INVALID)
                }
                _ => None,
            },
            ACC_BLOCK => self.acc_block(p),
            ACC_AFTER => {
                if self.ch(p) == b'=' && p < self.n {
                    return self.then(p, p + 1, ASSIGNMENT, &[LINE_IN]);
                }
                self.any_leave(p)
            }
            ATTR_IN => self.attr_in(p),
            ATTR_ARGS => self.shift(p, &[ATTR_IN]),
            ARGUMENTS => self.arguments(p),
            OUT_ARG => self.out_arg(p),
            ARG_EXPR => {
                if let Some(q) = self.nosemi(p) {
                    return Some(q);
                }
                if self.ch(p) == b';' && p < self.n {
                    return self.leave(p);
                }
                None
            }
            ACCESSOR_ARGS => self.accessor_args(p),
            LAMBDA_BODY => {
                if self.ch(p) == b';'
                    && p < self.n
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.line_in(p)
            }
            LAMBDA_PARAMS => self.lambda_params(p),
            MEMBER_VARS => self.member_vars(p),
            MV_VALUE => {
                if matches!(self.ch(p), b';' | b',')
                    && p < self.n
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.line_in(p)
            }
            VARS_DECL => self.vars_decl(p),
            VD_VALUE => {
                let c = self.ch(p);
                if matches!(c, b';' | b',')
                    && p < self.n
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                if c == b'{' {
                    return self.enter(p, &[VD_INIT]);
                }
                self.line_in(p)
            }
            VD_INIT => {
                if self.init_stop(p)
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.initializer(p)
            }
            INIT_CTOR => self.initializer(p),
            NEW => {
                if self.ch(p) == b'{' {
                    return self.then(p, p + 1, BLOCK_BEGIN, &[ANON_CLASS]);
                }
                if self.nonspace(p) {
                    return self.shift(p, &[NEW_TYPE]);
                }
                None
            }
            ANON_CLASS => self.anon_class(p),
            NEW_TYPE => self.new_type(p),
            NEW_BRACKETS => match self.ch(p) {
                b',' if p < self.n => self.tok_push(p, p + 1, ARGUMENT_SEP, &[LINE_IN]),
                b']' => self.close(p, p + 1, BRACKETS_END),
                _ => self.any_enter(p, &[LINE_IN]),
            },
            CTOR_ARGS => {
                let c = self.ch(p);
                if self.nonspace(p) && c != b'{' {
                    return self.leave(p);
                }
                (c == b'{').then_some(())?;
                self.then(p, p + 1, BRACES_BEGIN, &[INIT_CTOR])
            }
            TERNARY => {
                if self.ch(p) == b':' && p < self.n {
                    return self.close(p, p + 1, TERNARY_OP);
                }
                self.line_in(p)
            }
            CAST => self.cast(p),
            TUPLE => self.tuple(p),
            GEN_TYPES => match self.ch(p) {
                b',' if p < self.n => self.tok(p, p + 1, TYPE_SEP),
                b'>' => self.close(p, p + 1, GENERIC_END),
                _ => self.type_(p),
            },
            GROUP => {
                if self.ch(p) == b')' {
                    return self.close(p, p + 1, GROUP_END);
                }
                if let Some(q) = self.lambdas(p) {
                    return Some(q);
                }
                self.nosemi(p)
            }
            GROUP_IN => {
                if self.ch(p) == b')' {
                    return self.close(p, p + 1, GROUP_END);
                }
                self.line_in(p)
            }
            CODE_BRACE => {
                if self.ch(p) == b'}' {
                    return self.close(p, p + 1, BLOCK_END);
                }
                self.code_block_in(p)
            }
            EB_BRACE => {
                if self.ch(p) == b'}' {
                    return self.close(p, p + 1, M_BLOCK_END);
                }
                self.code_block_in(p)
            }
            N_GENERIC_CALL => self.n_generic_call(p),
            NAMEOF => self.nameof(p),
            TYPEOF => {
                if self.ch(p) == b')' {
                    return self.close(p, p + 1, M_GROUP_END);
                }
                self.type_(p)
            }
            CHECKED => match self.ch(p) {
                b'(' => self.then(p, p + 1, GROUP_BEGIN, &[GROUP_IN]),
                b'{' => self.then(p, p + 1, BLOCK_BEGIN, &[CODE_BRACE]),
                _ if self.nonspace(p) => {
                    let e = self.next(p);
                    self.tok(p, e, EXPECTED_BLOCK)
                }
                _ => None,
            },
            UNSAFE => match self.ch(p) {
                b'{' => self.then(p, p + 1, BLOCK_BEGIN, &[CODE_BRACE]),
                _ if self.nonspace(p) => {
                    let e = self.next(p);
                    self.tok(p, e, EXPECTED_BLOCK)
                }
                _ => None,
            },
            LOC_GENERIC => {
                if self.ch(p) == b'>' {
                    return self.then(p, p + 1, GENERIC_END, &[LOC_AFTER_GENERIC]);
                }
                self.type_arg_param_common(p)
            }
            LOC_AFTER_GENERIC => {
                if self.ch(p) == b'.' {
                    return self.then(p, p + 1, DOT_NAMESPACE, &[LINE]);
                }
                if let Some(e) = self.sp1(p) {
                    self.fill(e);
                    self.set(&[VARS_DECL]);
                    return Some(e);
                }
                self.any_shift(p, &[LINE])
            }
            NESTED_METHOD => {
                if let Some(q) = self.method_decl(p) {
                    return Some(q);
                }
                self.leave(p)
            }
            TYPE_NO_SPACE => {
                if let Some(q) = self.type_(p) {
                    return Some(q);
                }
                if self.space(p) {
                    let e = self.next(p);
                    return self.close(p, e, PLAIN);
                }
                None
            }
            TYPE_ARG => self.type_arg_param_common(p),
            TUPLE_T => match self.ch(p) {
                b')' => self.close(p, p + 1, GROUP_END),
                b',' if p < self.n => self.tok(p, p + 1, TUPLE_SEP),
                _ => self.any_enter(p, &[VDE]),
            },
            VDE => self.vde(p),
            VDE_GENERIC => {
                if self.ch(p) == b'>' {
                    return self.then(p, p + 1, GENERIC_END, &[VDE_NAME]);
                }
                self.type_arg_param_common(p)
            }
            VDE_NAME => {
                if let Some(e) = self.name(p) {
                    self.put(p, e, VAR_OTHER);
                    let s = self.sp(e);
                    self.fill(s);
                    self.pop();
                    return Some(s);
                }
                if matches!(self.ch(p), b',' | b')' | b';' | b'}') && p < self.n {
                    return self.leave(p);
                }
                None
            }
            VAR_DECL => self.var_decl(p),
            VD_TUPLE => match self.ch(p) {
                b')' => self.close(p, p + 1, TUPLE_GROUP_END),
                b'_' if !self.word_at(p + 1) => self.tok(p, p + 1, DISCARD),
                b',' if p < self.n => self.tok(p, p + 1, TUPLE_SEP),
                _ => {
                    let e = self.name(p)?;
                    self.tok(p, e, VAR_OTHER)
                }
            },
            FOR_BLOCK => {
                (self.ch(p) == b')').then_some(())?;
                self.then(p, p + 1, GROUP_END, &[FOR_AFTER])
            }
            FOR_AFTER | WHILE_BLOCK => {
                if self.ch(p) == b'{' {
                    return self.then(p, p + 1, BLOCK_BEGIN, &[CODE_BRACE]);
                }
                self.any_shift(p, &[LINE])
            }
            FOR_VAR_ASSIGN => match self.ch(p) {
                b'=' if p < self.n => self.then(p, p + 1, LOOP_ASSIGN, &[LINE_IN]),
                b';' | b')' | b'}' if p < self.n => self.leave(p),
                _ => None,
            },
            FOREACH_VAR_ASSIGN => {
                if self.kw(p, b"in") {
                    return self.then(p, p + 2, FLOW, &[LINE_IN]);
                }
                if matches!(self.ch(p), b')' | b'}') {
                    return self.leave(p);
                }
                None
            }
            IF_COND => {
                let s = self.sp(p);
                if self.ch(s) == b'(' && s < self.n {
                    self.put(s, s + 1, M_GROUP_BEGIN);
                    self.set(&[IF_GROUP]);
                    return Some(s + 1);
                }
                if p <= self.n && self.ch(p) != b'(' {
                    return self.leave(p);
                }
                None
            }
            IF_GROUP => {
                let s = self.sp(p);
                if self.ch(s) == b')' && s < self.n {
                    self.put(s, s + 1, M_GROUP_END);
                    self.pop();
                    return Some(s + 1);
                }
                self.line_in(p)
            }
            IF_BLOCK => {
                if self.ch(p) == b'{' {
                    return self.then(p, p + 1, M_BLOCK_BEGIN, &[MAIN_BRACE_M]);
                }
                self.any_shift(p, &[IF_LINE])
            }
            IF_LINE => {
                if self.lit(p, b"else")
                    && !self.word_at(p + 4)
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.line(p)
            }
            ELSE_BLOCK => self.else_block(p),
            ELSE_BODY => {
                let s = self.sp(p);
                if self.ch(s) == b'{' && s < self.n {
                    self.put(s, s + 1, M_BLOCK_BEGIN);
                    self.set(&[MAIN_BRACE_M]);
                    return Some(s + 1);
                }
                self.any_shift(p, &[LINE])
            }
            SWITCH_BLOCK => {
                if self.ch(p) == b'{' {
                    return self.then(p, p + 1, BLOCK_BEGIN, &[SW_BRACE]);
                }
                self.any_leave(p)
            }
            SW_BRACE => self.sw_brace(p),
            SW_CASE => self.sw_case(p),
            TRY_BLOCK => {
                if self.ch(p) == b'{' {
                    return self.then(p, p + 1, BLOCK_BEGIN, &[MAIN_BRACE]);
                }
                self.any_shift(p, &[TRY_LINE])
            }
            TRY_LINE => {
                let s = self.sp_line(p);
                if s >= self.n {
                    if s > p {
                        self.fill(s);
                        self.pop();
                        return Some(s);
                    }
                    if let Some(q) = self.leave(p) {
                        return Some(q);
                    }
                }
                self.main(p)
            }
            CATCH_EXPR => {
                if self.lit(p, b"catch") {
                    let s = self.sp(p + 5);
                    if self.ch(s) == b'(' && s < self.n {
                        self.put(p, p + 5, CATCH_KW);
                        self.put(s, s + 1, M_GROUP_BEGIN);
                        self.push(CATCH_BLOCK);
                        self.push(VDE);
                        return Some(s + 1);
                    }
                    return self.tok_push(p, p + 5, CATCH_KW, &[TRYCATCH_BLOCK]);
                }
                self.any_leave(p)
            }
            CATCH_BLOCK => {
                (self.ch(p) == b')').then_some(())?;
                self.then(p, p + 1, M_GROUP_END, &[CATCH_WHEN])
            }
            CATCH_WHEN => {
                if self.kw(p, b"when") {
                    return self.then(p, p + 4, TRYCATCH_WHEN, &[CW_OPEN]);
                }
                self.any_shift(p, &[TRYCATCH_BLOCK])
            }
            CW_OPEN => {
                if self.ch(p) == b'(' {
                    return self.then(p, p + 1, GROUP_BEGIN, &[CW_GROUP]);
                }
                self.any_shift(p, &[TRYCATCH_BLOCK])
            }
            CW_GROUP => {
                if self.ch(p) == b')' {
                    return self.then(p, p + 1, GROUP_END, &[TRYCATCH_BLOCK]);
                }
                self.line_in(p)
            }
            FINALLY_BLOCK => {
                if self.kw_end(p, b"finally") {
                    return self.then(p, p + 7, FINALLY_KW, &[TRYCATCH_BLOCK]);
                }
                self.any_leave(p)
            }
            TRYCATCH_BLOCK | UB_AFTER => {
                if self.ch(p) == b'{' {
                    return self.then(p, p + 1, BLOCK_BEGIN, &[CODE_BRACE]);
                }
                self.any_leave(p)
            }
            USING_BLOCK => {
                if self.ch(p) == b')' {
                    return self.then(p, p + 1, M_GROUP_END, &[UB_AFTER]);
                }
                self.any_leave(p)
            }
            EXPR_BLOCK => {
                (self.ch(p) == b')').then_some(())?;
                self.then(p, p + 1, M_GROUP_END, &[EB_AFTER])
            }
            EB_AFTER => {
                if self.ch(p) == b'{' {
                    return self.then(p, p + 1, M_BLOCK_BEGIN, &[EB_BRACE]);
                }
                self.any_shift(p, &[LINE])
            }
            DO_COND => {
                if self.kw(p, b"while") {
                    return self.then(p, p + 5, WHILE_KW, &[WHILE_COND]);
                }
                self.any_leave(p)
            }
            WHILE_COND => {
                if self.ch(p) == b'(' {
                    return self.then(p, p + 1, GROUP_BEGIN, &[GROUP_IN]);
                }
                self.any_leave(p)
            }
            PREPROC => self.preproc(p),
            PP_IF => self.pp_if(p),
            OPTION_DONE => self.option_done(p),
            PP_CHECKSUM => {
                if self.lit(p, b"\"{") {
                    return self.tok_push(p, p + 2, HASH_BEGIN, &[PP_HASH]);
                }
                if self.ch(p) == b'"' && p < self.n {
                    return self.open(p, p + 1, STRING_BEGIN, &[STRING]);
                }
                self.option_done(p)
            }
            PP_HASH => {
                let mut e = p;
                while e < self.n && (self.b[e] == b'-' || self.b[e].is_ascii_hexdigit()) {
                    e += 1;
                }
                if e > p {
                    return self.tok(p, e, HASH_HEX);
                }
                if self.lit(p, b"}\"") {
                    return self.close(p, p + 2, HASH_END);
                }
                if self.ch(p) == b'.' {
                    return self.close(p, p + 1, HASH_INVALID);
                }
                None
            }
            PP_WARNING => self.pp_warning(p),
            DOC => self.doc(p),
            DOC_TAG => self.doc_tag(p),
            LINE_COMMENT => {
                if p == self.n {
                    let e = self.next(p);
                    return self.close(p, e, PLAIN);
                }
                if p > self.n {
                    return self.leave(p);
                }
                self.comments_in(p, LC_TODO, LC_LINK)
            }
            BLOCK_COMMENT => {
                if self.lit(p, b"*/") {
                    return self.close(p, p + 2, BC_END);
                }
                if p == 0 {
                    let s = self.sp_line(0);
                    if self.ch(s) == b'*' && s < self.n && self.ch(s + 1) != b'/' {
                        return self.tok(s, s + 1, BC_STAR);
                    }
                }
                self.comments_in(p, BC_TODO, BC_LINK)
            }
            STRING => self.string(p),
            LONG_STRING => self.long_string(p),
            FORMAT_STRING | FORMAT_STRING_LOST => self.format_string(p),
            LONG_FORMAT_STRING | LONG_FORMAT_STRING_LOST => self.long_format_string(p),
            S_PH => self.placeholder(p, false),
            R_PH => self.placeholder(p, true),
            FS_S | FS_I => self.format_spec(p),
            FS_R | FS_RI => self.long_format_spec(p),
            INTERP => {
                if p >= self.n
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                if let Some(q) = self.placeholder_format(p, false, FS_I) {
                    return Some(q);
                }
                if self.ch(p) == b'}' {
                    return self.close(p, p + 1, INTERP_END);
                }
                self.line_in(p)
            }
            LINTERP => {
                if let Some(q) = self.placeholder_format(p, true, FS_RI) {
                    return Some(q);
                }
                if self.ch(p) == b'}' {
                    return self.close(p, p + 1, INTERP_END);
                }
                self.line_in(p)
            }
            _ => self.in_regex(p, top),
        }
    }

    /// In a regex in a string (`top` from [`REGEX`]), at `p`: the string's end and escapes (a
    /// verbatim one's `""`) but `\\`, which is the regex's, else the regex to the next place
    /// they can start.
    #[cold]
    #[inline(never)]
    fn in_regex(&mut self, p: usize, top: u8) -> Option<usize> {
        if top < REGEX {
            return None;
        }
        let verbatim = self.stack[self.depth - 2] == LONG_STRING;
        if p >= self.n {
            if verbatim {
                return None;
            }
            // unclosed at the line's end: the string's
            self.pop();
            return self.string(p);
        }
        let b = self.b;
        match b[p] {
            b'"' if verbatim && b.get(p + 1) == Some(&b'"') => return self.tok(p, p + 2, ESCAPE),
            b'"' => {
                self.pop();
                return self.close(p, p + 1, STRING_END);
            }
            b'\\' if !verbatim && b.get(p + 1) != Some(&b'\\') => return self.string_escaped(p),
            _ => {}
        }
        static YIELDS: [[bool; 256]; 2] =
            [crate::regex::table(b"\"\\"), crate::regex::table(b"\"")];
        let host = crate::regex::Embed {
            dialect: crate::regex::Dialect::DotNet,
            cooked: !verbatim,
            yields: &YIELDS[usize::from(verbatim)],
            stop: |b, p, _| b[p] == b'"',
            arg: 0,
            escape: escape_end,
            top_only: false,
            continues: false,
            base: self.cur() | RE_LEAF,
            text: self.cur(),
        };
        let (q, st) = crate::regex::embedded(b, p, top - REGEX, &host, &mut self.at, self.out)?;
        self.stack[self.depth - 1] = REGEX + st;
        Some(q)
    }

    /// A string opened at `p` (its `"`, or the `@` of `@"`): a regex in it where a call makes
    /// it one, by the `(` or `,` before it.
    fn regex_in(&mut self, p: usize) {
        let b = self.b;
        let q = blank_back(b, p);
        let regex = q > 0
            && match b[q - 1] {
                b'(' => regex_first_argument(b, q),
                b',' => self
                    .regex_calls
                    .get_or_insert_with(|| RegexCalls::find(b))
                    .has_comma(q - 1),
                _ => false,
            };
        if regex && self.depth < SYNTECT_DEPTH {
            self.push(REGEX);
        }
    }

    /// `(?=\S)` at `p`: a pop.
    fn any_leave(&mut self, p: usize) -> Option<usize> {
        if self.nonspace(p) {
            return self.leave(p);
        }
        None
    }

    /// `(?=\S)` at `p`: a push of `cs`.
    fn any_enter(&mut self, p: usize, cs: &[u8]) -> Option<usize> {
        if self.nonspace(p) {
            return self.enter(p, cs);
        }
        None
    }

    /// `(?=\S)` at `p`: a set.
    fn any_shift(&mut self, p: usize, cs: &[u8]) -> Option<usize> {
        if self.nonspace(p) {
            return self.shift(p, cs);
        }
        None
    }

    /// `stray_close_bracket`: `[})\]]`.
    fn stray(&mut self, p: usize) -> Option<usize> {
        if matches!(self.ch(p), b'}' | b')' | b']') && p < self.n {
            return self.tok(p, p + 1, STRAY);
        }
        None
    }

    /// `code_block_in`: `(?=\S)`, a statement.
    fn code_block_in(&mut self, p: usize) -> Option<usize> {
        self.any_enter(p, &[LINE])
    }

    /// `main`.
    fn main(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.stray(p) {
            return Some(q);
        }
        if self.ch(p) == b'{' {
            return self.tok_push(p, p + 1, BLOCK_BEGIN, &[MAIN_BRACE]);
        }
        if let Some(q) = self.attribute(p) {
            return Some(q);
        }
        if let Some(q) = self.using(p) {
            return Some(q);
        }
        if let Some(q) = self.class_decl(p) {
            return Some(q);
        }
        if let Some(q) = self.interface_decl(p) {
            return Some(q);
        }
        if self.kw(p, b"namespace") {
            return self.tok_push(p, p + 9, NAMESPACE_KW, &[NS_DECL]);
        }
        if let Some(q) = self.delegate_decl(p) {
            return Some(q);
        }
        self.any_enter(p, &[MAIN_LINE])
    }

    /// `{ … }` holding `main`, its `}` `end`.
    fn brace_main(&mut self, p: usize, end: u16) -> Option<usize> {
        if self.ch(p) == b'}' {
            return self.close(p, p + 1, end);
        }
        self.main(p)
    }

    /// `#anon_main_1`'s pop: `(?={{visibility}}|\b(?:class|delegate|interface|namespace|readonly)\b)`.
    fn main_line_stop(&self, p: usize) -> bool {
        self.visibility(p).is_some()
            || self
                .kw_any(
                    p,
                    &[
                        b"class",
                        b"delegate",
                        b"interface",
                        b"namespace",
                        b"readonly",
                    ],
                )
                .is_some()
    }

    /// `attribute`.
    fn attribute(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) != b'[' || p >= self.n {
            return None;
        }
        let s = self.sp(p + 1);
        for w in [
            &b"assembly"[..],
            b"module",
            b"field",
            b"event",
            b"method",
            b"param",
            b"property",
            b"return",
            b"type",
        ] {
            if self.lit(s, w) {
                let t = self.sp(s + w.len());
                if self.ch(t) == b':' && t < self.n {
                    self.fill(p);
                    self.push(ATTR_IN);
                    self.put(p, p + 1, ANNOTATION_BEGIN);
                    self.put(s, s + w.len(), TARGET);
                    self.put(t, t + 1, KEY_VALUE);
                    return Some(t + 1);
                }
            }
        }
        self.open(p, p + 1, ANNOTATION_BEGIN, &[ATTR_IN])
    }

    /// `using`.
    fn using(&mut self, p: usize) -> Option<usize> {
        if !self.kw_end(p, b"using") || self.word_before(p) {
            // `\b(using)\s+` needs no `\b` after, but `\s+` there is one
            return None;
        }
        let e = p + 5;
        // `\b(using)\s+(?={{name}}\s*=\s*)`
        if let Some(s) = self.sp1(e) {
            let mut alias = false;
            self.names(s, &mut |n| {
                let t = self.sp(n);
                alias = self.ch(t) == b'=' && t < self.n;
                alias
            });
            if alias {
                self.put(p, e, IMPORT);
                self.fill(s);
                self.push(USING_NS);
                return Some(s);
            }
        }
        // `\b(using)(?!\s*\()(?:\s+(static))?\b`
        let s = self.sp(e);
        if self.ch(s) == b'(' && s < self.n {
            return None;
        }
        if let Some(s) = self.sp1(e)
            && self.kw_end(s, b"static")
        {
            self.put(p, e, IMPORT);
            self.put(s, s + 6, IMPORT);
            self.push(USING_NS);
            return Some(s + 6);
        }
        self.tok_push(p, e, IMPORT, &[USING_NS])
    }

    /// `using_namespace`.
    fn using_ns(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.namespace_alias(p, NAMESPACE_VAR) {
            return Some(q);
        }
        if let Some(e) = self.name(p) {
            return self.tok(p, e, PATH);
        }
        match self.ch(p) {
            b'=' if p < self.n => return self.tok(p, p + 1, ASSIGNMENT),
            b'.' => return self.tok(p, p + 1, NS_SEP),
            b'<' => return self.tok_push(p, p + 1, GENERIC_BEGIN, &[TYPE_ARG]),
            b';' => return self.close(p, p + 1, TERMINATOR),
            _ => {}
        }
        if self.nonspace(p) {
            let mut e = p;
            while self.nonspace(e) && self.b[e] != b';' {
                e = self.next(e);
            }
            return self.tok(p, e, EXPECTED_NAMESPACE);
        }
        if self.eol(p) {
            return self.leave(p);
        }
        None
    }

    /// `namespace_alias`: `(?:(global)|({{name}}))\s*(::)`, the name `name_leaf`.
    fn namespace_alias(&mut self, p: usize, name_leaf: u16) -> Option<usize> {
        if self.lit(p, b"global") {
            let s = self.sp(p + 6);
            if self.lit(s, b"::") {
                self.put(p, p + 6, SUPPORT_NAMESPACE);
                return self.tok(s, s + 2, DOUBLE_COLON);
            }
        }
        let mut at = None;
        self.names(p, &mut |e| {
            let s = self.sp(e);
            if self.lit(s, b"::") {
                at = Some((e, s));
                return true;
            }
            false
        });
        let (e, s) = at?;
        self.put(p, e, name_leaf);
        self.tok(s, s + 2, DOUBLE_COLON)
    }

    /// `#anon_namespace_declaration_0`.
    fn ns_decl(&mut self, p: usize) -> Option<usize> {
        if self.name(p).is_some() {
            return self.enter(p, &[NS_NAME]);
        }
        if self.ch(p) == b'{' {
            return self.then(p, p + 1, BLOCK_BEGIN, &[MAIN_BRACE]);
        }
        self.any_leave(p)
    }

    /// `#anon_namespace_declaration_1`: the name, in `entity.name.namespace`.
    fn ns_name(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.name(p) {
            return self.tok(p, e, PLAIN);
        }
        if self.ch(p) == b'.' {
            return self.tok(p, p + 1, NS_SEP);
        }
        self.leave(p)
    }

    /// `class_declaration`.
    fn class_decl(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.kw_any(
            p,
            &[b"static", b"unsafe", b"abstract", b"partial", b"sealed"],
        ) {
            return self.tok(p, e, MODIFIER);
        }
        if let Some(e) = self.visibility(p) {
            return self.tok(p, e, ACCESS);
        }
        if self.kw_end(p, b"class")
            && !self.word_before(p)
            && let Some(s) = self.sp1(p + 5)
            && let Some(e) = self.name(s)
        {
            self.put(p, p + 5, CLASS_KW);
            self.put(s, e, CLASS_NAME);
            self.push(DATA_TYPE_BODY);
            self.push(DATA_TYPE_SIG);
            return Some(e);
        }
        if let Some(q) = self.struct_decl(p) {
            return Some(q);
        }
        if self.kw_end(p, b"enum")
            && !self.word_before(p)
            && let Some(s) = self.sp1(p + 4)
            && let Some(e) = self.name(s)
        {
            let t = self.sp(e);
            let mut end = t;
            let mut typed = None;
            if self.ch(t) == b':' && t < self.n {
                let u = self.sp(t + 1);
                for w in [
                    &b"byte"[..],
                    b"sbyte",
                    b"short",
                    b"ushort",
                    b"int",
                    b"uint",
                    b"long",
                    b"ulong",
                ] {
                    if self.lit(u, w) {
                        typed = Some((t, u, u + w.len()));
                        end = u + w.len();
                        break;
                    }
                }
            }
            self.put(p, p + 4, ENUM_KW);
            self.put(p + 4, s, ENUM);
            self.put(s, e, ENUM_NAME);
            if let Some((t, u, v)) = typed {
                self.put(e, t, ENUM);
                self.put(t, t + 1, ENUM_SEP);
                self.put(t + 1, u, ENUM);
                self.put(u, v, ENUM_TYPE);
            } else {
                self.put(e, end, ENUM);
            }
            self.push(ENUM_HEAD);
            return Some(end);
        }
        None
    }

    /// `(?:\b(readonly)\s+)?(?:\b(ref)\s+)?\b(struct)\s+({{name}})`.
    fn struct_decl(&mut self, p: usize) -> Option<usize> {
        let opt = |s: &Self, q: usize, w: &[u8]| -> Option<usize> {
            if s.lit(q, w) && !s.word_before(q) {
                s.sp1(q + w.len())
            } else {
                None
            }
        };
        if !matches!(self.ch(p), b'r' | b's') {
            return None;
        }
        let mut starts = [(None, None, 0); 4];
        let mut k = 0;
        if let Some(r) = opt(self, p, b"readonly") {
            if let Some(f) = opt(self, r, b"ref") {
                starts[k] = (Some(p), Some(r), f);
                k += 1;
            }
            starts[k] = (Some(p), None, r);
            k += 1;
        }
        if let Some(f) = opt(self, p, b"ref") {
            starts[k] = (None, Some(p), f);
            k += 1;
        }
        starts[k] = (None, None, p);
        k += 1;
        for &(ro, rf, q) in &starts[..k] {
            if self.lit(q, b"struct")
                && !self.word_before(q)
                && let Some(s) = self.sp1(q + 6)
                && let Some(e) = self.name(s)
            {
                if let Some(r) = ro {
                    self.put(r, r + 8, MODIFIER);
                }
                if let Some(r) = rf {
                    self.put(r, r + 3, MODIFIER);
                }
                self.put(q, q + 6, STRUCT_KW);
                self.put(s, e, STRUCT_NAME);
                self.push(DATA_TYPE_BODY);
                self.push(DATA_TYPE_SIG);
                return Some(e);
            }
        }
        None
    }

    /// `interface_declaration`: `(interface)\s+({{name}})`.
    fn interface_decl(&mut self, p: usize) -> Option<usize> {
        if !self.lit(p, b"interface") {
            return None;
        }
        let s = self.sp1(p + 9)?;
        let e = self.name(s)?;
        self.put(p, p + 9, INTERFACE_KW);
        self.put(s, e, INTERFACE_NAME);
        self.push(DATA_TYPE_BODY);
        self.push(DATA_TYPE_SIG);
        Some(e)
    }

    /// `delegate_declaration`: `\b(delegate)\b\s+`.
    fn delegate_decl(&mut self, p: usize) -> Option<usize> {
        if !self.kw(p, b"delegate") {
            return None;
        }
        let s = self.sp1(p + 8)?;
        self.put(p, p + 8, DELEGATE_KW);
        self.fill(s);
        self.push(DELEGATE_TYPE);
        Some(s)
    }

    /// `#anon_class_declaration_2`, an enum's body.
    fn enum_body(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b'}' {
            return self.then(p, p + 1, BLOCK_END, &[ENUM_END]);
        }
        if let Some(q) = self.attribute(p) {
            return Some(q);
        }
        if let Some(e) = self.name(p) {
            return self.tok(p, e, ENUM_CONSTANT);
        }
        match self.ch(p) {
            b'=' if p < self.n => self.tok_push(p, p + 1, ASSIGNMENT, &[LINE_IN]),
            b',' if p < self.n => self.tok(p, p + 1, ENUM_COMMA),
            _ => None,
        }
    }

    /// `delegate_name`.
    fn delegate_name(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.sp1(p) {
            return self.tok(p, e, DELEGATE);
        }
        if let Some(e) = self.name(p) {
            let s = self.sp(e);
            match self.ch(s) {
                b'<' if s < self.n => {
                    self.put(p, e, DELEGATE_NAME_LEAF);
                    self.put(e, s, DELEGATE);
                    return self.then(s, s + 1, DELEGATE_GENERIC_BEGIN, &[DN_GENERIC]);
                }
                b'(' if s < self.n => {
                    self.put(p, e, DELEGATE_NAME_LEAF);
                    self.put(e, s, DELEGATE);
                    return self.then(s, s + 1, DELEGATE_PARAMS_BEGIN, &[DELEGATE_PARAMS]);
                }
                _ => {}
            }
        }
        self.any_leave(p)
    }

    /// `delegate_end`.
    fn delegate_end(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b';' && p < self.n {
            return self.close(p, p + 1, TERMINATOR);
        }
        if p < self.n && self.space(p) {
            let s = self.sp_line(p);
            if s < self.n {
                return self.close(p, s, EXPECTED_COLON);
            }
            return None;
        }
        if self.nonspace(p) {
            let e = self.next(p);
            return self.close(p, e, EXPECTED_COLON);
        }
        None
    }

    /// `data_type_signature`.
    fn data_type_sig(&mut self, p: usize) -> Option<usize> {
        match self.ch(p) {
            b'<' => return self.tok_push(p, p + 1, GENERIC_BEGIN, &[DTS_GENERIC]),
            b':' if p < self.n => return self.then(p, p + 1, TYPE_SEP, &[TYPE_CONSTRAINT]),
            b'{' => return self.leave(p),
            _ => {}
        }
        if self.kw_end(p, b"where") {
            return self.shift(p, &[TYPE_CONSTRAINT]);
        }
        None
    }

    /// `data_type_body`.
    fn data_type_body(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b'{' {
            return self.tok_push(p, p + 1, BLOCK_BEGIN, &[DTB_BLOCK]);
        }
        if p > self.n {
            return self.leave(p);
        }
        let mut e = p;
        while self.nonspace(e) {
            e = self.next(e);
        }
        if e > p {
            return self.close(p, e, ILLEGAL);
        }
        self.leave(p)
    }

    /// `#anon_data_type_body_0`: a class's body.
    fn dtb_block(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b'}' {
            return self.close(p, p + 1, BLOCK_END);
        }
        if let Some(q) = self.attribute(p) {
            return Some(q);
        }
        if let Some(q) = self.class_decl(p) {
            return Some(q);
        }
        if let Some(q) = self.interface_decl(p) {
            return Some(q);
        }
        if let Some(q) = self.delegate_decl(p) {
            return Some(q);
        }
        if let Some(q) = self.method_decl(p) {
            return Some(q);
        }
        self.stray(p)
    }

    /// `type_constraint`.
    fn type_constraint(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b'{'
            && let Some(q) = self.leave(p)
        {
            return Some(q);
        }
        // `\b(where)\s+(?:({{base_type}})|({{name}}))\s*(:)`
        if self.kw_end(p, b"where")
            && !self.word_before(p)
            && let Some(s) = self.sp1(p + 5)
        {
            let mut hit = None;
            if let Some(e) = self.base_type(s) {
                let t = self.sp(e);
                if self.ch(t) == b':' && t < self.n {
                    hit = Some((e, t, STORAGE_TYPE));
                }
            }
            if hit.is_none() {
                self.names(s, &mut |e| {
                    let t = self.sp(e);
                    if self.ch(t) == b':' && t < self.n {
                        hit = Some((e, t, SUPPORT_TYPE));
                        return true;
                    }
                    false
                });
            }
            if let Some((e, t, leaf)) = hit {
                self.put(p, p + 5, MODIFIER);
                self.put(s, e, leaf);
                return self.tok(t, t + 1, TYPE_SEP);
            }
        }
        if let Some(q) = self.namespace_alias(p, PATH) {
            return Some(q);
        }
        for w in [&b"class"[..], b"struct", b"interface"] {
            if self.lit(p, w) {
                return self.tok(p, p + w.len(), STORAGE_TYPE);
            }
        }
        if let Some(e) = self.new_parens(p) {
            return self.tok(p, e, NEW_OP);
        }
        if let Some(e) = self.base_type(p) {
            return self.tok(p, e, STORAGE_TYPE);
        }
        // `({{name}})?(\.)`
        let mut dot = None;
        self.names(p, &mut |e| {
            if self.ch(e) == b'.' && e < self.n {
                dot = Some(e);
                return true;
            }
            false
        });
        if let Some(e) = dot {
            self.put(p, e, PATH);
            return self.tok(e, e + 1, DOT_NAMESPACE);
        }
        match self.ch(p) {
            b'.' => return self.tok(p, p + 1, DOT_NAMESPACE),
            b',' if p < self.n => return self.tok(p, p + 1, INHERITED_SEP),
            _ => {}
        }
        if let Some(e) = self.name(p) {
            return self.tok(p, e, INHERITED);
        }
        self.type_(p)
    }

    /// `new\s*\(\s*\)` at `p`: where it ends.
    fn new_parens(&self, p: usize) -> Option<usize> {
        if !self.lit(p, b"new") {
            return None;
        }
        let s = self.sp(p + 3);
        (self.ch(s) == b'(' && s < self.n).then_some(())?;
        let t = self.sp(s + 1);
        (self.ch(t) == b')' && t < self.n).then_some(t + 1)
    }

    /// `event_handler_declaration`.
    fn event_handler(&mut self, p: usize) -> Option<usize> {
        let mut dot = None;
        self.names(p, &mut |e| {
            if self.ch(e) == b'.' && e < self.n {
                dot = Some(e);
                return true;
            }
            false
        });
        if let Some(e) = dot {
            self.put(p, e, INHERITED);
            return self.tok(e, e + 1, DOT);
        }
        if let Some(e) = self.name(p) {
            return self.then(p, e, MEMBER, &[EH_AFTER]);
        }
        self.any_leave(p)
    }

    /// `method_declaration`.
    fn method_decl(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.kw_any(
            p,
            &[
                b"abstract",
                b"async",
                b"const",
                b"extern",
                b"new",
                b"override",
                b"readonly",
                b"ref",
                b"sealed",
                b"static",
                b"unsafe",
                b"virtual",
                b"volatile",
            ],
        ) {
            return self.tok(p, e, MODIFIER);
        }
        if self.kw(p, b"event") {
            let e = self.sp_line(p + 5);
            let e = if e == self.n { e + 1 } else { e };
            self.put(p, p + 5, MODIFIER);
            self.fill(e);
            self.push(EVENT_HANDLER);
            self.push(TYPE_NO_SPACE);
            return Some(e);
        }
        if self.kw(p, b"delegate") {
            return self.tok(p, p + 8, DELEGATE_KW);
        }
        if let Some(e) = self.kw_any(p, &[b"implicit", b"explicit"]) {
            return self.tok(p, e, MODIFIER);
        }
        if let Some(e) = self.visibility(p) {
            return self.tok(p, e, ACCESS);
        }
        // `({{name}})\s*(<)`
        if let Some((e, s)) = self.name_then(p, b'<') {
            self.put(p, e, SUPPORT_TYPE);
            self.put(s, s + 1, GENERIC_BEGIN);
            self.push(METHOD_NAME);
            self.push(TYPE_ARG);
            return Some(s + 1);
        }
        // `(~{{name}})(\s*)(\()`
        if self.ch(p) == b'~'
            && let Some((e, s)) = self.name_then(p + 1, b'(')
        {
            self.put(p, e, DTOR);
            self.put(e, s, METHOD);
            self.put(s, s + 1, PARAMS_BEGIN);
            self.push(CTOR_PREBODY);
            self.push(METHOD_PARAMS);
            return Some(s + 1);
        }
        // `({{name}})\s*(\()`
        if let Some((e, s)) = self.name_then(p, b'(') {
            self.put(p, e, CTOR);
            self.put(s, s + 1, PARAMS_BEGIN);
            self.push(CTOR_PREBODY);
            self.push(METHOD_PARAMS);
            return Some(s + 1);
        }
        // `({{name}})\s*(\.)`
        if let Some((e, s)) = self.name_then(p, b'.') {
            self.put(p, e, INHERITED);
            return self.tok(s, s + 1, DOT);
        }
        // `\b(?:({{base_type}})|({{name}}))`
        if self.bnd(p) {
            if let Some(e) = self.base_type(p) {
                return self.tok_push(p, e, STORAGE_TYPE, &[MD_TYPE]);
            }
            if let Some(e) = self.name(p) {
                return self.tok_push(p, e, SUPPORT_TYPE, &[MD_TYPE]);
            }
        }
        if self.kw(p, b"operator") {
            return self.tok_push(p, p + 8, MODIFIER, &[METHOD_NAME]);
        }
        if self.ch(p) == b'(' {
            return self.tok_push(p, p + 1, GROUP_BEGIN, &[METHOD_NAME, TUPLE_T]);
        }
        None
    }

    /// `({{name}})\s*(c)` at `p`, the name's ends tried in order: the name's end and `c`'s
    /// place.
    fn name_then(&self, p: usize, c: u8) -> Option<(usize, usize)> {
        let mut at = None;
        self.names(p, &mut |e| {
            let s = self.sp(e);
            if self.ch(s) == c && s < self.n {
                at = Some((e, s));
                return true;
            }
            false
        });
        at
    }

    /// The tokens of a `{{type_suffix_capture}}` (`method_declaration`'s, after a type), the
    /// leaf of the `>` before it if it had one.
    fn suffix_tokens(&mut self, x: &re::Suffix, _gt: Option<u16>) {
        if let Some(q) = x.question {
            self.put(q - 1, q, NULLABLE);
        }
        if let Some((l, r)) = x.brackets {
            self.put(l, l + 1, M_BRACKETS_BEGIN);
            self.put(l + 1, r, BRACKETS_SEP);
            self.put(r, r + 1, M_BRACKETS_END);
        }
        if let Some(s) = x.star {
            self.put(s, s + 1, POINTER);
        }
        self.fill(x.end);
    }

    /// `method_name`: after a member's type, its name: a method's, a property's, a field's.
    fn method_name(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b'.' {
            return self.tok(p, p + 1, DOT);
        }
        // `(operator)\s*({{unary_op}}|{{bin_op}})\s*(\()`
        if self.lit(p, b"operator") {
            let s = self.sp(p + 8);
            for op in UNARY_OP.iter().chain(BIN_OP.iter()) {
                if self.lit(s, op) {
                    let t = self.sp(s + op.len());
                    if self.ch(t) == b'(' && t < self.n {
                        self.put(p, p + 8, MODIFIER_OTHER);
                        self.put(s, s + op.len(), METHOD_FUNCTION);
                        self.put(t, t + 1, PARAMS_BEGIN);
                        self.set(&[METHOD_BODY, METHOD_PARAMS]);
                        return Some(t + 1);
                    }
                }
            }
        }
        // `({{name}}\s*)?((=>)\s*)`
        let mut arrow = None;
        self.names(p, &mut |e| {
            let s = self.sp(e);
            if self.lit(s, b"=>") {
                arrow = Some((Some(e), s));
                return true;
            }
            false
        });
        if arrow.is_none() && self.lit(p, b"=>") {
            arrow = Some((None, p));
        }
        if let Some((name, s)) = arrow {
            if name.is_some() {
                self.put(p, s, MEMBER);
            }
            self.put(s, s + 2, M_GET_ARROW);
            let t = self.sp(s + 2);
            self.put(s + 2, t, METHOD);
            self.set(&[LINE_IN]);
            return Some(t);
        }
        // `({{name}})\s*(?==[^>])`
        let mut assign = None;
        self.names(p, &mut |e| {
            let s = self.sp(e);
            if self.ch(s) == b'=' && s < self.n && self.ch(s + 1) != b'>' && self.on(s + 1) {
                assign = Some((e, s));
                return true;
            }
            false
        });
        if let Some((e, s)) = assign {
            self.put(p, e, MEMBER);
            self.fill(s);
            self.set(&[MEMBER_VARS]);
            return Some(s);
        }
        // `({{name}})?\s*(\()`
        let call = self
            .name_then(p, b'(')
            .map(|(e, s)| (Some(e), s))
            .or_else(|| {
                let s = self.sp(p);
                (self.ch(s) == b'(' && s < self.n).then_some((None, s))
            });
        if let Some((name, s)) = call {
            if let Some(e) = name {
                self.put(p, e, METHOD_FUNCTION);
            }
            self.put(s, s + 1, PARAMS_BEGIN);
            self.set(&[METHOD_BODY, METHOD_PARAMS]);
            return Some(s + 1);
        }
        // `({{name}})\s*(;)`
        if let Some((e, s)) = self.name_then(p, b';') {
            self.put(p, e, MEMBER);
            return self.close(s, s + 1, TERM_STMT);
        }
        // `({{name}})\s*(<)(?=[^(]*>\.)`, then `({{name}})\s*(<)`
        if let Some((e, s)) = self.name_then(p, b'<') {
            if self.gt_dot_ahead(s + 1) {
                self.put(p, e, INHERITED_M);
                self.put(e, s, METHOD);
                return self.tok_push(s, s + 1, SECTION_GENERIC_BEGIN, &[MN_EXPLICIT]);
            }
            self.put(p, e, ENTITY_FUNCTION);
            return self.tok_push(s, s + 1, GENERIC_BEGIN, &[MN_GENERIC]);
        }
        // `(this)\s*(\[)`
        if self.lit(p, b"this") {
            let s = self.sp(p + 4);
            if self.ch(s) == b'[' && s < self.n {
                self.put(p, p + 4, VAR_LANGUAGE);
                self.put(s, s + 1, M_BRACKETS_BEGIN);
                self.set(&[METHOD_ACCESSOR, METHOD_PARAMS_BRACKET]);
                return Some(s + 1);
            }
        }
        // `({{name}})\s*(\.)`
        if let Some((e, s)) = self.name_then(p, b'.') {
            self.put(p, e, INHERITED_M);
            self.put(e, s, METHOD);
            return self.tok(s, s + 1, DOT_M);
        }
        if let Some(e) = self.name(p) {
            return self.tok_push(p, e, PROPERTY_MEMBER, &[MN_AFTER]);
        }
        // `(?=\s*\{)`
        let s = self.sp(p);
        if self.ch(s) == b'{' && s < self.n {
            return self.shift(p, &[METHOD_ACCESSOR]);
        }
        None
    }

    /// `(?=[^(]*>\.)` at `i`.
    fn gt_dot_ahead(&self, i: usize) -> bool {
        let mut q = i;
        while self.on(q) && self.ch(q) != b'(' {
            if self.ch(q) == b'>' && self.ch(q + 1) == b'.' && q + 1 < self.n {
                return true;
            }
            q += 1;
        }
        false
    }

    /// `#anon_method_name_1`: an explicit interface's type arguments.
    fn mn_explicit(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b'>' {
            let s = self.sp(p + 1);
            if self.ch(s) == b'.' && s < self.n {
                self.put(p, p + 1, GENERIC_END_P);
                self.put(p + 1, s, METHOD_GENERIC);
                return self.close(s, s + 1, DOT_P);
            }
        }
        if self.ch(p) == b',' && p < self.n {
            return self.tok(p, p + 1, PARAM_TYPE_SEP);
        }
        self.type_(p)
    }

    /// `method_body`.
    fn method_body(&mut self, p: usize) -> Option<usize> {
        if self.kw_end(p, b"where") {
            return self.enter(p, &[TYPE_CONSTRAINT]);
        }
        if self.lit(p, b"=>") {
            return self.then(p, p + 2, FUNCTION_ARROW, &[LINE_IN]);
        }
        match self.ch(p) {
            b'{' => return self.then(p, p + 1, BLOCK_BEGIN, &[MB_BLOCK]),
            b';' if p < self.n => return self.close(p, p + 1, TERMINATOR),
            _ => {}
        }
        if self.nonspace(p) {
            let mut e = p;
            while self.nonspace(e) {
                e = self.next(e);
            }
            return self.close(p, e, INVALID);
        }
        None
    }

    /// `method_param`.
    fn method_param(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.name(p) {
            return self.tok(p, e, PARAMETER);
        }
        match self.ch(p) {
            b'=' if p < self.n => self.tok_push(p, p + 1, DEFAULT_VALUE, &[LINE_IN]),
            b',' if p < self.n => self.close(p, p + 1, PARAMETER_SEP),
            b'}' | b')' | b'>' | b']' | b';' if p < self.n => self.leave(p),
            _ => None,
        }
    }

    /// `#anon_method_param_type_0`: a parameter's type.
    fn mpt_type(&mut self, p: usize) -> Option<usize> {
        let s = self.sp(p);
        if !self.word_before(s) || s == p {
            for w in [&b"out"[..], b"ref", b"this", b"params", b"in"] {
                if self.lit(s, w)
                    && !self.word_before(s)
                    && let Some(t) = self.sp1(s + w.len())
                {
                    self.put(s, s + w.len(), PARAM_MODIFIER);
                    self.fill(t);
                    return Some(t);
                }
            }
        }
        if self.space(p) {
            let e = self.next(p);
            return self.close(p, e, PLAIN);
        }
        self.type_(p)
    }

    /// `#anon_constructor_prebody_0`.
    fn ctor_colon(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.sp1(p) {
            return self.tok(p, e, PREBODY);
        }
        for w in [&b"this"[..], b"base"] {
            if self.lit(p, w) {
                let s = self.sp(p + 4);
                if self.ch(s) == b'(' && s < self.n {
                    self.put(p, p + 4, VAR_LANGUAGE);
                    self.put(s, s + 1, M_GROUP_BEGIN);
                    self.set(&[CTOR_INIT, ARGUMENTS]);
                    return Some(s + 1);
                }
            }
        }
        if self.ch(p) == b'{' {
            return self.shift(p, &[METHOD_BODY]);
        }
        None
    }

    /// `#anon_method_accessor_0`: a property's `{ … }`.
    fn acc_block(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b'}' {
            return self.then(p, p + 1, BLOCK_END, &[ACC_AFTER]);
        }
        if self.kw(p, b"get") {
            return self.tok_push(p, p + 3, M_GET, &[METHOD_BODY]);
        }
        if self.kw(p, b"set") {
            return self.tok_push(p, p + 3, SET_KW, &[METHOD_BODY]);
        }
        if let Some(e) = self.visibility(p) {
            return self.tok(p, e, ACCESS);
        }
        self.attribute(p)
    }

    /// `attribute_in`.
    fn attr_in(&mut self, p: usize) -> Option<usize> {
        // `({{name}})(\()`
        let mut call = None;
        self.names(p, &mut |e| {
            if self.ch(e) == b'(' && e < self.n {
                call = Some(e);
                return true;
            }
            false
        });
        if let Some(e) = call {
            self.put(p, e, VARIABLE_ANNOTATION);
            self.put(e, e + 1, M_GROUP_BEGIN);
            self.set(&[ATTR_ARGS, ARGUMENTS]);
            return Some(e + 1);
        }
        if let Some((e, s)) = self.name_then(p, b'.') {
            self.put(p, e, NAMESPACE_VAR);
            return self.tok(s, s + 1, DOT_NAMESPACE);
        }
        if let Some(q) = self.namespace_alias(p, SUPPORT_NAMESPACE) {
            return Some(q);
        }
        if let Some(e) = self.name(p) {
            return self.tok(p, e, VARIABLE_ANNOTATION);
        }
        match self.ch(p) {
            b',' if p < self.n => self.tok(p, p + 1, ANNOTATION_SEP),
            b']' => self.close(p, p + 1, ANNOTATION_END),
            _ => None,
        }
    }

    /// `arguments`.
    fn arguments(&mut self, p: usize) -> Option<usize> {
        if self.lit(p, b"out") && self.space(p + 3) {
            let e = self.next(p + 3);
            self.put(p, p + 3, ARG_MODIFIER);
            self.fill(e);
            self.push(OUT_ARG);
            return Some(e);
        }
        if self.lit(p, b"ref") && self.space(p + 3) {
            let e = self.next(p + 3);
            self.put(p, p + 3, ARG_MODIFIER);
            self.fill(e);
            return Some(e);
        }
        if let Some(e) = self.ascii_name(p) {
            // the name's one end: `({{name}})\s*(=)(?=[^>=])`, `({{name}})\s*(:)`, else an
            // expression
            let s = self.sp(e);
            if s < self.n {
                let c = self.b[s];
                let leaf = match c {
                    b'=' if self.on(s + 1) && !matches!(self.ch(s + 1), b'>' | b'=') => {
                        Some(ASSIGNMENT)
                    }
                    b':' => Some(ARG_VALUE_SEP),
                    _ => None,
                };
                if let Some(leaf) = leaf {
                    self.put(p, e, PARAMETER);
                    return self.tok_push(s, s + 1, leaf, &[LINE_IN]);
                }
            }
            return self.enter(p, &[ARG_EXPR]);
        }
        // `({{name}})\s*(=)(?=[^>=])`, then `({{name}})\s*(:)`
        let mut hit = None;
        self.names(p, &mut |e| {
            let s = self.sp(e);
            if self.ch(s) == b'='
                && s < self.n
                && self.on(s + 1)
                && !matches!(self.ch(s + 1), b'>' | b'=')
            {
                hit = Some((e, s, ASSIGNMENT));
                return true;
            }
            false
        });
        if hit.is_none() {
            self.names(p, &mut |e| {
                let s = self.sp(e);
                if self.ch(s) == b':' && s < self.n {
                    hit = Some((e, s, ARG_VALUE_SEP));
                    return true;
                }
                false
            });
        }
        if let Some((e, s, leaf)) = hit {
            self.put(p, e, PARAMETER);
            return self.tok_push(s, s + 1, leaf, &[LINE_IN]);
        }
        match self.ch(p) {
            b',' if p < self.n => return self.tok(p, p + 1, ARGUMENT_SEP),
            b')' => return self.close(p, p + 1, GROUP_END),
            b';' if p < self.n => return self.close(p, p + 1, EXPECTED_CLOSE_PAREN),
            _ => {}
        }
        if let Some(q) = self.stray(p) {
            return Some(q);
        }
        self.any_enter(p, &[ARG_EXPR])
    }

    /// `#anon_arguments_0`: after `out `.
    fn out_arg(&mut self, p: usize) -> Option<usize> {
        if self.lit(p, b"ref") && self.space(p + 3) {
            let e = self.next(p + 3);
            self.put(p, p + 3, ARG_MODIFIER);
            self.fill(e);
            return Some(e);
        }
        if !self.declaration(p, &mut |_| true)
            && let Some(q) = self.leave(p)
        {
            return Some(q);
        }
        self.var_decl(p)
    }

    /// `accessor_arguments`.
    fn accessor_args(&mut self, p: usize) -> Option<usize> {
        if let Some((e, s)) = self.name_then(p, b':') {
            self.put(p, e, PARAMETER_OTHER);
            return self.tok(s, s + 1, KEY_VALUE_PARAMETER);
        }
        match self.ch(p) {
            b',' if p < self.n => return self.tok(p, p + 1, ACCESSOR_SEP),
            b']' => return self.close(p, p + 1, M_BRACKETS_END),
            _ => {}
        }
        self.line_in(p)
    }

    /// `#anon_lambdas_1`: a lambda's `( … ) =>`.
    fn lambda_params(&mut self, p: usize) -> Option<usize> {
        let mut param = None;
        self.names(p, &mut |e| {
            let s = self.sp(e);
            if matches!(self.ch(s), b')' | b',' | b'/') && s < self.n {
                param = Some(e);
                return true;
            }
            false
        });
        if let Some(e) = param {
            return self.tok(p, e, PARAMETER);
        }
        if self.ch(p) == b',' && p < self.n {
            return self.tok(p, p + 1, PARAMETER_SEP);
        }
        if self.ch(p) == b')' {
            let s = self.sp(p + 1);
            if self.lit(s, b"=>") {
                self.put(p, p + 1, M_GROUP_END);
                self.put(s, s + 2, LAMBDA);
                self.set(&[LAMBDA_BODY]);
                return Some(s + 2);
            }
        }
        None
    }

    /// `member_variables_declaration`.
    fn member_vars(&mut self, p: usize) -> Option<usize> {
        match self.ch(p) {
            b'=' if p < self.n => return self.tok_push(p, p + 1, ASSIGN_VARIABLE, &[MV_VALUE]),
            b'{' => return self.enter(p, &[METHOD_ACCESSOR]),
            _ => {}
        }
        if let Some(e) = self.name(p) {
            return self.tok(p, e, MEMBER);
        }
        match self.ch(p) {
            b',' if p < self.n => self.tok(p, p + 1, VARIABLES_SEP),
            b';' if p < self.n => self.close(p, p + 1, TERM_STMT),
            b'}' | b')' | b'>' | b']' if p < self.n => self.leave(p),
            _ => None,
        }
    }

    /// `variables_declaration`.
    fn vars_decl(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b'=' && p < self.n {
            return self.tok_push(p, p + 1, ASSIGN_VARIABLE, &[VD_VALUE]);
        }
        if let Some(e) = self.name(p) {
            return self.tok(p, e, VAR_OTHER);
        }
        match self.ch(p) {
            b',' if p < self.n => self.tok(p, p + 1, VARIABLES_SEP),
            b';' if p < self.n => self.close(p, p + 1, TERM_STMT),
            b'{' | b'}' | b')' | b'>' | b']' if p < self.n => self.leave(p),
            _ => None,
        }
    }

    /// `(?=[^,\s{}])` at `p`.
    fn init_stop(&self, p: usize) -> bool {
        self.nonspace(p) && !matches!(self.b[p], b',' | b'{' | b'}')
    }

    /// `initializer_constructor`.
    fn initializer(&mut self, p: usize) -> Option<usize> {
        match self.ch(p) {
            b'}' => return self.close(p, p + 1, BRACES_END_I),
            b'{' => return self.tok_push(p, p + 1, BRACES_BEGIN, &[INIT_CTOR]),
            b',' if p < self.n => return self.tok(p, p + 1, ARRAY_SEP),
            _ => {}
        }
        if self.init_stop(p) {
            return self.enter(p, &[LINE_IN]);
        }
        None
    }

    /// `#anon_maybe_an_anonymous_class_0`.
    fn anon_class(&mut self, p: usize) -> Option<usize> {
        match self.ch(p) {
            b'}' => return self.close(p, p + 1, BLOCK_END),
            b',' if p < self.n => return self.tok(p, p + 1, CLASS_SEP),
            _ => {}
        }
        if let Some((e, s)) = self.name_then(p, b'=') {
            self.put(p, e, MEMBER);
            return self.tok_push(s, s + 1, ASSIGNMENT, &[LINE_IN]);
        }
        self.any_enter(p, &[LINE_IN])
    }

    /// `#anon_maybe_an_anonymous_class_1`: after `new`, not an anonymous class.
    fn new_type(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.brackets_capture(p) {
            return Some(q);
        }
        if self.ch(p) == b'[' && p < self.n {
            return self.tok_push(p, p + 1, BRACKETS_BEGIN, &[NEW_BRACKETS]);
        }
        // `(?:\s*((\()\s*(\)))\s*)?(\{)`
        let s = self.sp(p);
        if self.ch(s) == b'(' && s < self.n {
            let t = self.sp(s + 1);
            if self.ch(t) == b')' && t < self.n {
                let u = self.sp(t + 1);
                if self.ch(u) == b'{' && u < self.n {
                    self.put(p, s, INSTANCE);
                    self.put(s, s + 1, INSTANCE_GROUP_BEGIN);
                    self.put(s + 1, t, INSTANCE_GROUP);
                    self.put(t, t + 1, INSTANCE_GROUP_END);
                    self.put(t + 1, u, INSTANCE);
                    return self.then(u, u + 1, INSTANCE_BRACES_BEGIN, &[INIT_CTOR]);
                }
            }
        }
        if self.ch(p) == b'{' {
            return self.then(p, p + 1, INSTANCE_BRACES_BEGIN, &[INIT_CTOR]);
        }
        if self.ch(p) == b'(' {
            return self.then(p, p + 1, M_GROUP_BEGIN, &[CTOR_ARGS, ARGUMENTS]);
        }
        self.type_(p)
    }

    /// `{{brackets_capture}}`, `((\[)(,*)(\]))`, as a token.
    fn brackets_capture(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) != b'[' || p >= self.n {
            return None;
        }
        let mut r = p + 1;
        while r < self.n && self.b[r] == b',' {
            r += 1;
        }
        if r < self.n && self.b[r] == b']' {
            self.put(p, p + 1, M_BRACKETS_BEGIN);
            self.put(p + 1, r, BRACKETS_SEP);
            return self.tok(r, r + 1, M_BRACKETS_END);
        }
        None
    }

    /// `#anon_line_of_code_in_no_semicolon_3`: a cast's `( … )`.
    fn cast(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b')' {
            return self.close(p, p + 1, GROUP_END);
        }
        if let Some(e) = self.base_type(p) {
            return self.tok(p, e, STORAGE_TYPE);
        }
        if let Some(e) = self.name(p) {
            return self.tok(p, e, SUPPORT_TYPE);
        }
        if self.ch(p) == b'.' {
            return self.tok(p, p + 1, NS_SEP);
        }
        if p == self.step {
            let x = self.type_suffix_capture(p);
            if x.end > p {
                // bat's captures here are off by one group
                if let Some(q) = x.question {
                    self.put(q - 1, q, BRACKETS_BEGIN);
                }
                if let Some((l, r)) = x.brackets {
                    self.put(l, l + 1, CAST_BRACKETS_SEP);
                    self.put(l + 1, r, CAST_POINTER);
                    self.put(r, r + 1, CAST_SEP);
                }
                self.fill(x.end);
                return Some(x.end);
            }
        }
        None
    }

    /// `#anon_line_of_code_in_no_semicolon_4`: a tuple's `( … )`.
    fn tuple(&mut self, p: usize) -> Option<usize> {
        match self.ch(p) {
            b')' => return self.close(p, p + 1, GROUP_END),
            b',' if p < self.n => return self.tok(p, p + 1, TUPLE_SEP),
            b':' if p < self.n => return self.tok(p, p + 1, ASSIGNMENT),
            b'_' if !self.word_at(p + 1) => return self.tok(p, p + 1, DISCARD),
            _ => {}
        }
        // `(?!{{reserved}})(?={{namespaced_name}}{{type_suffix}}\s+{{name}}\s*[:,])`
        if !self.starts_reserved(p)
            && self.declaration(p, &mut |n| {
                let s = self.sp(n);
                matches!(self.ch(s), b':' | b',') && s < self.n
            })
        {
            return self.enter(p, &[VDE]);
        }
        let mut generic = None;
        self.names(p, &mut |e| {
            if self.ch(e) == b'<' && e < self.n {
                generic = Some(e);
                return true;
            }
            false
        });
        if let Some(e) = generic {
            self.put(p, e, SUPPORT_TYPE);
            return self.tok_push(e, e + 1, GENERIC_BEGIN, &[GEN_TYPES]);
        }
        self.nosemi(p)
    }

    /// `#anon_line_of_code_in_no_semicolon_0`: a generic call's type arguments.
    fn n_generic_call(&mut self, p: usize) -> Option<usize> {
        match self.ch(p) {
            b',' if p < self.n => return self.tok(p, p + 1, TYPE_SEP),
            b'>' => {
                let s = self.sp(p + 1);
                if self.ch(s) == b'(' && s < self.n {
                    self.put(p, p + 1, FC_GENERIC_END);
                    self.put(p + 1, s, FUNCTION_CALL);
                    self.put(s, s + 1, FC_GROUP_BEGIN);
                    self.set(&[ARGUMENTS]);
                    return Some(s + 1);
                }
            }
            _ => {}
        }
        self.type_(p)
    }

    /// `#anon_keywords_0`: `nameof( … )`.
    fn nameof(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b')' {
            return self.close(p, p + 1, M_GROUP_END);
        }
        if let Some(e) = self.base_type(p) {
            return self.tok(p, e, STORAGE_TYPE);
        }
        if self.lowercase_at(p) {
            let mut hit = None;
            self.names(p, &mut |e| {
                let s = self.sp(e);
                if self.ch(s) == b')' && s < self.n {
                    hit = Some(e);
                    return true;
                }
                false
            });
            if let Some(e) = hit {
                return self.tok(p, e, VAR_OTHER);
            }
        }
        self.type_(p)
    }

    /// `(?=\p{Ll})` at `p`.
    fn lowercase_at(&self, p: usize) -> bool {
        let Some(&c) = self.b.get(p) else {
            return false;
        };
        if c < 0x80 {
            return c.is_ascii_lowercase();
        }
        std::str::from_utf8(&self.b[p..(p + re::char_len(c)).min(self.n)])
            .ok()
            .and_then(|s| s.chars().next())
            .is_some_and(char::is_lowercase)
    }

    /// `var_declaration`.
    fn var_decl(&mut self, p: usize) -> Option<usize> {
        if self.lit(p, b"var") {
            let s = self.sp(p + 3);
            if self.ch(s) == b'(' && s < self.n {
                self.put(p, p + 3, VAR_KW);
                return self.then(s, s + 1, TUPLE_GROUP_BEGIN, &[VD_TUPLE]);
            }
            if let Some(s) = self.sp1(p + 3)
                && let Some(e) = self.name(s)
            {
                self.put(p, p + 3, VAR_KW);
                self.put(s, e, VAR_OTHER);
                let t = self.sp(e);
                self.fill(t);
                self.pop();
                return Some(t);
            }
        }
        self.vde(p)
    }

    /// `var_declaration_explicit`.
    fn vde(&mut self, p: usize) -> Option<usize> {
        if let Some((e, s)) = self.name_then(p, b'.') {
            self.put(p, e, SUPPORT_TYPE);
            return self.tok(s, s + 1, DOT_NAMESPACE);
        }
        if let Some((e, s)) = self.name_then(p, b'<') {
            self.put(p, e, SUPPORT_TYPE);
            return self.then(s, s + 1, GENERIC_BEGIN, &[VDE_GENERIC]);
        }
        if let Some(e) = self.base_type(p) {
            self.put(p, e, STORAGE_TYPE);
            let x = self.type_suffix_capture(e);
            self.suffix_tokens(&x, None);
            self.set(&[VDE_NAME]);
            return Some(x.end);
        }
        if let Some(e) = self.name(p) {
            self.put(p, e, SUPPORT_TYPE);
            let x = self.type_suffix_capture(e);
            self.suffix_tokens(&x, None);
            let mut end = x.end;
            if let Some(s) = self.sp1(x.end)
                && let Some(m) = self.name(s)
            {
                self.put(s, m, VAR_OTHER);
                end = m;
            }
            let t = self.sp(end);
            self.fill(t);
            self.pop();
            return Some(t);
        }
        if self.ch(p) == b')' {
            return self.leave(p);
        }
        None
    }

    /// `else_block`.
    fn else_block(&mut self, p: usize) -> Option<usize> {
        if self.lit(p, b"else") {
            // `(else\s+if)\b\s*`
            if let Some(s) = self.sp1(p + 4)
                && self.kw_end(s, b"if")
            {
                self.put(p, s + 2, ELSEIF_KW);
                let t = self.sp(s + 2);
                self.fill(t);
                self.push(IF_BLOCK);
                self.push(IF_COND);
                return Some(t);
            }
            // `(else)\s*`
            let t = self.sp(p + 4);
            return self.then(p, t, ELSE_KW, &[ELSE_BODY]);
        }
        self.any_leave(p)
    }

    /// `#anon_switch_block_0`: a switch's `{ … }`.
    fn sw_brace(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b'}' {
            return self.close(p, p + 1, BLOCK_END);
        }
        if self.kw_end(p, b"default") && !self.word_before(p) {
            let s = self.sp(p + 7);
            if self.ch(s) == b':' && s < self.n {
                self.put(p, p + 7, CASE_KW);
                return self.tok(s, s + 1, CASE_SEP);
            }
        }
        if self.kw(p, b"case") {
            return self.tok_push(p, p + 4, CASE_KW, &[SW_CASE]);
        }
        self.code_block_in(p)
    }

    /// `#anon_switch_block_1`: a case's label.
    fn sw_case(&mut self, p: usize) -> Option<usize> {
        let mut label = None;
        self.names(p, &mut |e| {
            let s = self.sp(e);
            if self.ch(s) == b':' && s < self.n {
                label = Some(e);
                return true;
            }
            false
        });
        if let Some(e) = label {
            return self.tok(p, e, CONSTANT_OTHER);
        }
        if self.kw(p, b"when") {
            return self.tok(p, p + 4, WHEN_KW);
        }
        if self.declaration(p, &mut |n| {
            self.sp1(n).is_some_and(|s| self.kw_end(s, b"when"))
        }) {
            return self.enter(p, &[VDE]);
        }
        if self.ch(p) == b':' && p < self.n {
            return self.close(p, p + 1, CASE_SEP);
        }
        if let Some(q) = self.line_in(p) {
            return Some(q);
        }
        if self.eol(p) {
            return self.leave(p);
        }
        None
    }

    /// `line_of_code`.
    fn line(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.ascii_name(p)
            && !self.word_before(p)
        {
            return self.line_name(p, e);
        }
        let c = self.ch(p);
        if c.is_ascii_lowercase()
            && !self.word_before(p)
            && let Some(q) = self.line_keyword(p)
        {
            return Some(q);
        }
        if let Some(q) = self.keywords(p) {
            return Some(q);
        }
        if self.nested_method_ahead(p) {
            return self.enter(p, &[NESTED_METHOD]);
        }
        // `({{name}})\s*(<)(?=([^(={};]*>){{type_suffix}}\s+{{name}}{{generic_declaration}}\()`
        let mut generic = None;
        self.names(p, &mut |e| {
            let s = self.sp(e);
            if self.ch(s) == b'<' && s < self.n && self.generic_method_ahead(s + 1) {
                generic = Some((e, s));
                return true;
            }
            false
        });
        if let Some((e, s)) = generic {
            self.put(p, e, SUPPORT_TYPE);
            self.put(s, s + 1, GENERIC_BEGIN);
            self.push(METHOD_NAME);
            self.push(TYPE_ARG);
            return Some(s + 1);
        }
        // `({{name}})\s*(<)(?=[^(={};]*>\()`
        let mut call = None;
        self.names(p, &mut |e| {
            let s = self.sp(e);
            if self.ch(s) == b'<' && s < self.n && self.generic_call_ahead(s + 1) {
                call = Some((e, s));
                return true;
            }
            false
        });
        if let Some((e, s)) = call {
            self.put(p, e, FC_FUNCTION);
            self.put(e, s, FUNCTION_CALL);
            return self.tok_push(s, s + 1, FC_GENERIC_BEGIN, &[GEN_TYPES]);
        }
        // `({{name}})(<)`
        let mut generic = None;
        self.names(p, &mut |e| {
            if self.ch(e) == b'<' && e < self.n {
                generic = Some(e);
                return true;
            }
            false
        });
        if let Some(e) = generic {
            self.put(p, e, SUPPORT_TYPE);
            return self.then(e, e + 1, GENERIC_BEGIN, &[LOC_GENERIC]);
        }
        if self.kw(p, b"const") {
            return self.tok(p, p + 5, MODIFIER);
        }
        if let Some(q) = self.var_line(p) {
            return Some(q);
        }
        if let Some(q) = self.lambdas(p) {
            return Some(q);
        }
        if let Some(q) = self.local_declaration(p) {
            return Some(q);
        }
        // `({{name}})\s*(:)(?!:)`
        let mut label = None;
        self.names(p, &mut |e| {
            let s = self.sp(e);
            if self.ch(s) == b':' && s < self.n && self.ch(s + 1) != b':' {
                label = Some((e, s));
                return true;
            }
            false
        });
        if let Some((e, s)) = label {
            self.put(p, e, LABEL);
            return self.close(s, s + 1, SEPARATOR);
        }
        if let Some(q) = self.namespace_alias(p, NAMESPACE_VAR) {
            return Some(q);
        }
        // `({{name}})\s+({{name}})`
        let mut pair = None;
        self.names(p, &mut |e| {
            if let Some(s) = self.sp1(e)
                && let Some(m) = self.name(s)
            {
                pair = Some((e, s, m));
                return true;
            }
            false
        });
        if let Some((e, s, m)) = pair {
            self.put(p, e, SUPPORT_TYPE);
            return self.then(s, m, VAR_OTHER, &[VARS_DECL]);
        }
        if c == b'.' {
            return self.tok(p, p + 1, DOT);
        }
        self.any_shift(p, &[LINE_IN])
    }

    /// `line_of_code` at a name of ASCII that ends at `e` (its one end, so every pattern's
    /// name is it), not after a word character: the patterns that can start with a letter or
    /// `_`, in the grammar's order.
    fn line_name(&mut self, p: usize, e: usize) -> Option<usize> {
        let w = &self.b[p..e];
        if self.b[p].is_ascii_lowercase() {
            if let Some(q) = self.line_keyword(p) {
                return Some(q);
            }
            if let Some(q) = self.keywords(p) {
                return Some(q);
            }
        }
        if self.nested_method_ahead(p) {
            return self.enter(p, &[NESTED_METHOD]);
        }
        let s = self.sp(e);
        if self.ch(s) == b'<' && s < self.n {
            if self.generic_method_ahead(s + 1) {
                self.put(p, e, SUPPORT_TYPE);
                self.put(s, s + 1, GENERIC_BEGIN);
                self.push(METHOD_NAME);
                self.push(TYPE_ARG);
                return Some(s + 1);
            }
            if self.generic_call_ahead(s + 1) {
                self.put(p, e, FC_FUNCTION);
                self.put(e, s, FUNCTION_CALL);
                return self.tok_push(s, s + 1, FC_GENERIC_BEGIN, &[GEN_TYPES]);
            }
            if s == e {
                self.put(p, e, SUPPORT_TYPE);
                return self.then(e, e + 1, GENERIC_BEGIN, &[LOC_GENERIC]);
            }
        }
        match w {
            b"const" => return self.tok(p, e, MODIFIER),
            b"var" | b"dynamic" => {
                if let Some(q) = self.var_line(p) {
                    return Some(q);
                }
            }
            _ => {}
        }
        // `({{name}})\s+(=>)\s*`
        if s > e && s < self.n && self.lit(s, b"=>") {
            self.put(p, e, PARAMETER);
            self.put(s, s + 2, LAMBDA);
            let t = self.sp(s + 2);
            self.fill(t);
            self.push(LAMBDA_BODY);
            return Some(t);
        }
        if let Some(q) = self.local_declaration(p) {
            return Some(q);
        }
        // `({{name}})\s*(:)(?!:)`
        if self.ch(s) == b':' && s < self.n && self.ch(s + 1) != b':' {
            self.put(p, e, LABEL);
            return self.close(s, s + 1, SEPARATOR);
        }
        if let Some(q) = self.namespace_alias(p, NAMESPACE_VAR) {
            return Some(q);
        }
        // `({{name}})\s+({{name}})`
        if s > e
            && let Some(m) = self.name(s)
        {
            self.put(p, e, SUPPORT_TYPE);
            return self.then(s, m, VAR_OTHER, &[VARS_DECL]);
        }
        self.shift(p, &[LINE_IN])
    }

    /// `line_of_code`'s statements by keyword, at a word's start.
    fn line_keyword(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let word_end = {
            let mut e = p;
            while e < self.n && b[e].is_ascii_alphabetic() {
                e += 1;
            }
            e
        };
        let w = &b[p..word_end];
        let whole = !self.word_at(word_end);
        match w {
            b"if" if whole => {
                return self.then(p, word_end, IF_KW, &[ELSE_BLOCK, IF_BLOCK, IF_COND]);
            }
            b"switch" if whole => {
                return self.then(p, word_end, SWITCH_KW, &[SWITCH_BLOCK, IF_COND]);
            }
            b"for" => {
                let s = self.sp(p + 3);
                if self.ch(s) == b'(' && s < self.n {
                    let declares = {
                        let q = s + 1;
                        let mut ok = false;
                        self.names(q, &mut |e| {
                            ok = self.space(e) && self.ch(e + 1) != b'=';
                            ok
                        });
                        ok || (self.lit(q, b"var") && self.space(q + 3) && self.ch(q + 4) != b'=')
                    };
                    self.put(p, p + 3, FOR_KW);
                    self.put(s, s + 1, M_GROUP_BEGIN);
                    if declares {
                        self.set(&[FOR_BLOCK, LINE_IN, LINE_IN, FOR_VAR_ASSIGN, VAR_DECL]);
                    } else {
                        self.set(&[FOR_BLOCK, LINE_IN, LINE_IN, NOSEMI]);
                    }
                    return Some(s + 1);
                }
            }
            b"foreach" => {
                let s = self.sp(p + 7);
                if self.ch(s) == b'(' && s < self.n {
                    self.put(p, p + 7, FOREACH_KW);
                    self.put(s, s + 1, M_GROUP_BEGIN);
                    self.set(&[FOR_BLOCK, FOREACH_VAR_ASSIGN, VAR_DECL]);
                    return Some(s + 1);
                }
            }
            b"try" if whole => {
                return self.then(p, word_end, TRY_KW, &[FINALLY_BLOCK, CATCH_EXPR, TRY_BLOCK]);
            }
            b"using" | b"fixed" | b"lock" => {
                let s = self.sp(word_end);
                if self.ch(s) == b'(' && s < self.n {
                    let (leaf, block) = match w {
                        b"using" => (USING_KW, USING_BLOCK),
                        b"fixed" => (FIXED_KW, EXPR_BLOCK),
                        _ => (LOCK_KW, EXPR_BLOCK),
                    };
                    self.put(p, word_end, leaf);
                    self.put(s, s + 1, M_GROUP_BEGIN);
                    self.set(&[block, LINE]);
                    return Some(s + 1);
                }
            }
            b"do" if whole => {
                return self.then(p, word_end, DO_KW, &[DO_COND, WHILE_BLOCK]);
            }
            b"while" if whole => {
                return self.then(p, word_end, WHILE_KW, &[WHILE_BLOCK, WHILE_COND]);
            }
            _ => {}
        }
        if whole && matches!(w, b"else" | b"case" | b"catch" | b"finally") {
            return self.tok(p, word_end, UNEXPECTED_KW);
        }
        // `\b(return|yield\s+return)\b`, `\b(continue|break|yield\s+break)\b\s*(;)?`
        if whole && crate::same(w, b"return") {
            return self.then(p, word_end, RETURN_KW, &[LINE_IN]);
        }
        let mut brk = (whole && matches!(w, b"continue" | b"break")).then_some(word_end);
        if crate::same(w, b"yield")
            && whole
            && let Some(s) = self.sp1(word_end)
        {
            if self.kw_end(s, b"return") {
                return self.then(p, s + 6, RETURN_KW, &[LINE_IN]);
            }
            if self.kw_end(s, b"break") {
                brk = Some(s + 5);
            }
        }
        if let Some(e) = brk {
            self.put(p, e, BREAK_KW);
            let s = self.sp(e);
            let end = if self.ch(s) == b';' && s < self.n {
                self.put(s, s + 1, TERM_STMT);
                s + 1
            } else {
                s
            };
            self.fill(end);
            self.pop();
            return Some(end);
        }
        if whole && crate::same(w, b"throw") {
            return self.then(p, word_end, THROW_KW, &[LINE_IN]);
        }
        if crate::same(w, b"goto")
            && let Some(s) = self.sp1(word_end)
        {
            if self.kw_end(s, b"case") {
                self.put(p, word_end, GOTO_KW);
                return self.then(s, s + 4, CASE_KW, &[LINE_IN]);
            }
            if let Some((e, t)) = self.name_then(s, b';') {
                self.put(p, word_end, GOTO_KW);
                self.put(s, e, LABEL_CONSTANT);
                return self.close(t, t + 1, TERM_STMT);
            }
        }
        None
    }

    /// `(?=(?:\b(?:async|ref)\s+)?{{name}}(?:{{generic_declaration}})?{{type_suffix}}\s+{{name}}{{generic_declaration}}\()`.
    fn nested_method_ahead(&self, p: usize) -> bool {
        // a name of ASCII not `async` or `ref`, followed by none of what can follow it here
        if let Some(e) = self.ascii_name(p)
            && !matches!(&self.b[p..e], b"async" | b"ref")
            && !self.after_type(e, false)
        {
            return false;
        }
        let rest = |q: usize| {
            self.names(q, &mut |e| {
                self.generic_decl(e, &mut |g| {
                    self.type_suffix(g, &mut |t| {
                        self.spaced_name(t, &mut |m| {
                            self.generic_decl(m, &mut |h| self.ch(h) == b'(' && h < self.n)
                        })
                    })
                })
            })
        };
        for w in [&b"async"[..], b"ref"] {
            if self.lit(p, w)
                && !self.word_before(p)
                && let Some(s) = self.sp1(p + w.len())
                && rest(s)
            {
                return true;
            }
        }
        rest(p)
    }

    /// Whether the byte at `e`, after a type's first name, can go on to a declaration's
    /// `\s+{{name}}`: a space (the line's `\n` too), `<`, `[`, `*`, `?`, and `.` if `dotted`.
    fn after_type(&self, e: usize, dotted: bool) -> bool {
        e >= self.n
            || matches!(self.b[e], b'<' | b'[' | b'*' | b'?')
            || (dotted && self.b[e] == b'.')
            || self.space(e)
    }

    /// `(?=([^(={};]*>){{type_suffix}}\s+{{name}}{{generic_declaration}}\()` at `i`.
    fn generic_method_ahead(&self, i: usize) -> bool {
        let mut r = i;
        while self.on(r) && !matches!(self.ch(r), b'(' | b'=' | b'{' | b'}' | b';') {
            r += 1;
        }
        let mut g = r;
        while g > i {
            g -= 1;
            if self.ch(g) == b'>'
                && self.type_suffix(g + 1, &mut |t| {
                    self.spaced_name(t, &mut |m| {
                        self.generic_decl(m, &mut |h| self.ch(h) == b'(' && h < self.n)
                    })
                })
            {
                return true;
            }
        }
        false
    }

    /// `(?=[^(={};]*>\()` at `i`.
    fn generic_call_ahead(&self, i: usize) -> bool {
        let mut r = i;
        while self.on(r) && !matches!(self.ch(r), b'(' | b'=' | b'{' | b'}' | b';') {
            r += 1;
        }
        r < self.n && self.ch(r) == b'(' && r > i && self.ch(r - 1) == b'>'
    }

    /// `line_of_code`'s `var`/`dynamic` patterns.
    fn var_line(&mut self, p: usize) -> Option<usize> {
        let kw = if self.lit(p, b"var") {
            3
        } else if self.lit(p, b"dynamic") {
            7
        } else {
            return None;
        };
        if let Some(s) = self.sp1(p + kw) {
            // `(var|dynamic)\s+({{reserved}}|{{base_type}})\s*(=)`
            let mut bad = None;
            let w = self.word(s);
            if re::is_reserved(w) {
                let t = self.sp(s + w.len());
                if self.ch(t) == b'=' && t < self.n {
                    bad = Some((s + w.len(), t));
                }
            }
            if bad.is_none()
                && let Some(e) = self.base_type(s)
            {
                let t = self.sp(e);
                if self.ch(t) == b'=' && t < self.n {
                    bad = Some((e, t));
                }
            }
            if let Some((e, t)) = bad {
                self.put(p, p + kw, VAR_KW);
                self.put(s, e, INVALID);
                return self.tok(t, t + 1, ASSIGN_VARIABLE);
            }
            // `(var|dynamic)\s+({{name}})\s*(=)`
            if let Some((e, t)) = self.name_then(s, b'=') {
                self.put(p, p + kw, VAR_KW);
                self.put(s, e, VAR_OTHER);
                return self.then(t, t + 1, ASSIGN_VARIABLE, &[LINE_IN]);
            }
        }
        // `(?=var\s*\()`
        if kw == 3 {
            let s = self.sp(p + 3);
            if self.ch(s) == b'(' && s < self.n {
                return self.enter(p, &[VAR_DECL]);
            }
        }
        None
    }

    /// `line_of_code`'s two declarations of a local.
    fn local_declaration(&mut self, p: usize) -> Option<usize> {
        // with `ref ` first, then without
        let mut starts = [(None, p); 2];
        let mut k = 0;
        if self.lit(p, b"ref")
            && !self.word_before(p)
            && let Some(s) = self.sp1(p + 3)
        {
            starts[0] = (Some(p), s);
            k = 1;
        }
        starts[k] = (None, p);
        let starts = &starts[..=k];
        // `(?:\b(ref)\s+)?\b(?={{namespaced_name}}{{type_suffix}}\s+{{name}}(?!\s*(?:[(:])))`
        for &(r, q) in starts {
            if self
                .ascii_name(q)
                .is_some_and(|e| !self.after_type(e, true))
            {
                continue;
            }
            if self.bnd(q)
                && self.declaration(q, &mut |n| {
                    let s = self.sp(n);
                    !(matches!(self.ch(s), b'(' | b':') && s < self.n)
                })
            {
                if let Some(r) = r {
                    self.put(r, r + 3, MODIFIER);
                }
                self.fill(q);
                self.set(&[VARS_DECL, TYPE_NO_SPACE]);
                return Some(q);
            }
        }
        // `(?:\b(ref)\s+)?\b(?:({{base_type}})|({{name}}))(?={{type_suffix}}\s+{{name}}(?!\s*:))`
        let ahead = |s: &Self, e: usize| {
            s.type_suffix(e, &mut |t| {
                s.spaced_name(t, &mut |m| {
                    let u = s.sp(m);
                    !(s.ch(u) == b':' && u < s.n)
                })
            })
        };
        for &(r, q) in starts {
            if !self.bnd(q)
                || self
                    .ascii_name(q)
                    .is_some_and(|e| !self.after_type(e, false))
            {
                continue;
            }
            let mut hit = None;
            if let Some(e) = self.base_type(q)
                && ahead(self, e)
            {
                hit = Some((e, STORAGE_TYPE));
            }
            if hit.is_none() {
                self.names(q, &mut |e| {
                    if ahead(self, e) {
                        hit = Some((e, SUPPORT_TYPE));
                        return true;
                    }
                    false
                });
            }
            if let Some((e, leaf)) = hit {
                if let Some(r) = r {
                    self.put(r, r + 3, MODIFIER);
                }
                return self.then(q, e, leaf, &[VARS_DECL, TYPE_NO_SPACE]);
            }
        }
        None
    }

    /// `keywords`.
    fn keywords(&mut self, p: usize) -> Option<usize> {
        let c = self.ch(p);
        if !c.is_ascii_lowercase() || self.word_before(p) {
            return None;
        }
        // each pattern needs its word whole (`this[` and `nameof(` too): one match on it
        let w = self.word(p);
        let e = p + w.len();
        match w {
            b"this" | b"base" => {
                let s = self.sp(e);
                if self.ch(s) == b'[' && s < self.n {
                    self.put(p, e, VAR_LANGUAGE);
                    return self.tok_push(s, s + 1, M_BRACKETS_BEGIN, &[ACCESSOR_ARGS]);
                }
                self.tok(p, e, VAR_LANGUAGE)
            }
            b"await" => self.tok(p, e, AWAIT),
            b"nameof" if self.ch(e) == b'(' && e < self.n => {
                self.put(p, e, KW_OTHER);
                self.tok_push(e, e + 1, M_GROUP_BEGIN, &[NAMEOF])
            }
            b"typeof" | b"default" if self.ch(e) == b'(' && e < self.n => {
                self.put(p, e, REFLECTION);
                self.tok_push(e, e + 1, M_GROUP_BEGIN, &[TYPEOF])
            }
            b"as" | b"is" => {
                let s = self.sp1(e)?;
                self.put(p, e, REFLECTION);
                self.fill(s);
                self.push(TYPE_NO_SPACE);
                Some(s)
            }
            b"checked" | b"unchecked" => self.tok_push(p, e, KW_OTHER, &[CHECKED]),
            b"unsafe" => self.tok_push(p, e, KW_OTHER, &[UNSAFE]),
            b"new" => self.tok_push(p, e, INSTANCE_NEW, &[NEW]),
            b"public" | b"private" | b"protected" | b"internal" => self.tok(p, e, ACCESS),
            _ => None,
        }
    }

    /// `lambdas`.
    fn lambdas(&mut self, p: usize) -> Option<usize> {
        // `({{name}})\s+(=>)\s*`
        let mut arrow = None;
        self.names(p, &mut |e| {
            if let Some(s) = self.sp1(e)
                && self.lit(s, b"=>")
            {
                arrow = Some((e, s));
                return true;
            }
            false
        });
        if let Some((e, s)) = arrow {
            self.put(p, e, PARAMETER);
            self.put(s, s + 2, LAMBDA);
            let t = self.sp(s + 2);
            self.fill(t);
            self.push(LAMBDA_BODY);
            return Some(t);
        }
        // `\((?=[^()]*\)\s+=>)`
        if self.ch(p) == b'(' && p < self.n {
            let mut q = p + 1;
            while self.on(q) && !matches!(self.ch(q), b'(' | b')') {
                q += 1;
            }
            if q < self.n
                && self.ch(q) == b')'
                && let Some(s) = self.sp1(q + 1)
                && self.lit(s, b"=>")
            {
                return self.tok_push(p, p + 1, LAMBDA_GROUP_BEGIN, &[LAMBDA_PARAMS]);
            }
        }
        None
    }

    /// `line_of_code_in`.
    fn line_in(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.nosemi(p) {
            return Some(q);
        }
        if self.ch(p) == b';' && p < self.n {
            return self.close(p, p + 1, TERM_STMT);
        }
        None
    }

    /// `line_of_code_in_no_semicolon`.
    fn nosemi(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.ascii_name(p)
            && !self.word_before(p)
        {
            return self.nosemi_name(p, e);
        }
        let c = self.ch(p);
        // punctuation: only the patterns that can start with it, in the grammar's order
        if p < self.n {
            match c {
                b'.' => return self.tok(p, p + 1, DOT),
                b')' | b']' | b',' | b'}' => return self.leave(p),
                b'{' => return self.nosemi_brace(p),
                b'[' => {
                    self.put(p, p + 1, M_BRACKETS_BEGIN);
                    self.push(ACCESSOR_ARGS);
                    return Some(p + 1);
                }
                b'(' => {
                    if let Some(q) = self.lambdas(p) {
                        return Some(q);
                    }
                    return self.paren(p);
                }
                b';' | b':' | b'#' => return None,
                _ => {}
            }
        }
        if c.is_ascii_lowercase() && !self.word_before(p) {
            if self.kw_end(p, b"value") {
                return self.tok(p, p + 5, VALUE);
            }
            if let Some(q) = self.keywords(p) {
                return Some(q);
            }
            if let Some(e) = self.kw_any(p, &SQL_WORDS) {
                let s = self.sp(e);
                let blocked = s < self.n
                    && matches!(
                        self.ch(s),
                        b'.' | b',' | b';' | b'+' | b'=' | b'&' | b'|' | b'[' | b']' | b')'
                    );
                if !blocked {
                    return self.tok(p, e, SQL);
                }
            }
        }
        match c {
            b'?' if self.ch(p + 1) == b'.' && p + 1 < self.n => {
                return self.tok(p, p + 2, NULL_COALESCING);
            }
            b'.' => return self.tok(p, p + 1, DOT),
            b'-' if self.ch(p + 1) == b'>' && p + 1 < self.n => {
                return self.tok(p, p + 2, ARROW);
            }
            _ => {}
        }
        if let Some(e) = self.operator(p) {
            return self.tok(p, e, OPERATOR);
        }
        if c == b'=' && p < self.n {
            return self.tok(p, p + 1, ASSIGNMENT);
        }
        if let Some(q) = self.literals(p) {
            return Some(q);
        }
        if c == b'$' && self.ch(p + 1) == b'"' && p + 1 < self.n {
            return self.open(p, p + 2, STRING_BEGIN, &[FORMAT_STRING]);
        }
        if c == b'@' && self.ch(p + 1) == b'"' && p + 1 < self.n {
            let q = self.open(p, p + 2, STRING_BEGIN, &[LONG_STRING]);
            self.regex_in(p);
            return q;
        }
        if (self.lit(p, b"@$\"") || self.lit(p, b"$@\"")) && p + 2 < self.n {
            return self.open(p, p + 3, STRING_BEGIN, &[LONG_FORMAT_STRING]);
        }
        if let Some(q) = self.names_in_code(p) {
            return Some(q);
        }
        if let Some(q) = self.lambdas(p) {
            return Some(q);
        }
        // `\s*(\??\[)`
        let s = self.sp(p);
        if s < self.n {
            let e = if self.ch(s) == b'[' {
                Some(s + 1)
            } else if self.ch(s) == b'?' && self.ch(s + 1) == b'[' && s + 1 < self.n {
                Some(s + 2)
            } else {
                None
            };
            if let Some(e) = e {
                self.put(s, e, M_BRACKETS_BEGIN);
                self.fill(e);
                self.push(ACCESSOR_ARGS);
                return Some(e);
            }
        }
        if let Some(e) = self.base_type(p) {
            return self.tok(p, e, STORAGE_TYPE);
        }
        if let Some(e) = self.reserved_word(p) {
            return self.tok(p, e, KW_OTHER);
        }
        if let Some(q) = self.namespace_alias(p, SUPPORT_NAMESPACE) {
            return Some(q);
        }
        if let Some(e) = self.name(p) {
            return self.tok(p, e, VAR_OTHER);
        }
        match c {
            b'@' if p < self.n => return self.tok(p, p + 1, RESERVED_CHAR),
            b'?' if p < self.n => return self.tok_push(p, p + 1, TERNARY_OP, &[TERNARY]),
            b'(' if p < self.n => return self.paren(p),
            b'{' => return self.nosemi_brace(p),
            _ => {}
        }
        if (matches!(c, b'}' | b')' | b'>' | b']' | b',') && p < self.n) || self.starts_reserved(p)
        {
            return self.leave(p);
        }
        None
    }

    /// `line_of_code_in_no_semicolon`'s `\{`, a set.
    fn nosemi_brace(&mut self, p: usize) -> Option<usize> {
        // a `set` restores the scopes the context cleared before its token: in an
        // interpolation, the string's
        if matches!(self.top(), INTERP | LINTERP) && self.depth > 2 {
            let f = self.depth - 2;
            let under = self.bits[f] & CHAIN;
            self.put_kind(p, p + 1, (self.cur() & !CHAIN) | under | BLOCK_BEGIN);
            // and then pops those two, so the string's scopes are gone for good
            self.stack[f] = match self.stack[f] {
                FORMAT_STRING => FORMAT_STRING_LOST,
                LONG_FORMAT_STRING => LONG_FORMAT_STRING_LOST,
                c => c,
            };
            self.bits[f] = level_bits(self.bits[f - 1], self.stack[f]);
            self.set(&[CODE_BRACE]);
            return Some(p + 1);
        }
        self.then(p, p + 1, BLOCK_BEGIN, &[CODE_BRACE])
    }

    /// `line_of_code_in_no_semicolon` at a name of ASCII that ends at `e` (its one end, so
    /// every pattern's name is it), not after a word character: the patterns that can start
    /// with a letter or `_`, in the grammar's order.
    fn nosemi_name(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let w = &b[p..e];
        let lower = b[p].is_ascii_lowercase();
        if lower {
            if crate::same(w, b"value") {
                return self.tok(p, e, VALUE);
            }
            if let Some(q) = self.keywords(p) {
                return Some(q);
            }
            if SQL_WORDS.iter().any(|x| crate::same(x, w)) {
                let s = self.sp(e);
                let blocked = s < self.n
                    && matches!(
                        self.ch(s),
                        b'.' | b',' | b';' | b'+' | b'=' | b'&' | b'|' | b'[' | b']' | b')'
                    );
                if !blocked {
                    return self.tok(p, e, SQL);
                }
            }
            if matches!(w, b"true" | b"false" | b"null") {
                return self.tok(p, e, LANGUAGE);
            }
        } else if b[p] == b'_'
            && let Some(q) = self.float(p)
        {
            return Some(q);
        }
        let after = self.ch(e);
        if after == b'<' && e < self.n {
            // `({{name}})(<)(?=[^(]*>\s*\()`, `({{name}})(<)(?={{namespaced_name}}>\s*\.)`
            if self.generic_call_paren_ahead(e + 1) {
                self.put(p, e, VAR_FUNCTION);
                return self.tok_push(e, e + 1, GENERIC_BEGIN, &[N_GENERIC_CALL]);
            }
            if self.namespaced(e + 1, &mut |g| {
                self.ch(g) == b'>' && {
                    let s = self.sp(g + 1);
                    self.ch(s) == b'.' && s < self.n
                }
            }) {
                self.put(p, e, SUPPORT_TYPE);
                return self.tok_push(e, e + 1, GENERIC_BEGIN, &[GEN_TYPES]);
            }
        }
        let s = self.sp(e);
        let next = self.ch(s);
        if s < self.n {
            // `((?!{{reserved}}\b){{name}})\s*(\()`
            if next == b'(' && !(lower && re::is_reserved(w)) {
                self.put(p, e, FC_FUNCTION);
                self.put(e, s, FUNCTION_CALL);
                return self.tok_push(s, s + 1, FC_GROUP_BEGIN, &[ARGUMENTS]);
            }
            // `({{name}})\s*(\??\[)`
            let open = if next == b'[' {
                Some(s + 1)
            } else if next == b'?' && self.ch(s + 1) == b'[' && s + 1 < self.n {
                Some(s + 2)
            } else {
                None
            };
            if let Some(o) = open {
                self.put(p, e, VAR_OTHER);
                return self.tok_push(s, o, M_BRACKETS_BEGIN, &[ACCESSOR_ARGS]);
            }
            // `({{name}})\s+(=>)\s*`
            if s > e && self.lit(s, b"=>") {
                self.put(p, e, PARAMETER);
                self.put(s, s + 2, LAMBDA);
                let t = self.sp(s + 2);
                self.fill(t);
                self.push(LAMBDA_BODY);
                return Some(t);
            }
        }
        if lower {
            if let Some(end) = self.base_type(p) {
                return self.tok(p, end, STORAGE_TYPE);
            }
            if re::is_reserved(w) {
                return self.tok(p, e, KW_OTHER);
            }
        }
        // `(?:(global)|({{name}}))\s*(::)`
        if self.lit(p, b"global") {
            let g = self.sp(p + 6);
            if self.lit(g, b"::") {
                self.put(p, p + 6, SUPPORT_NAMESPACE);
                return self.tok(g, g + 2, DOUBLE_COLON);
            }
        }
        if self.lit(s, b"::") {
            self.put(p, e, SUPPORT_NAMESPACE);
            return self.tok(s, s + 2, DOUBLE_COLON);
        }
        self.tok(p, e, VAR_OTHER)
    }

    /// `{{bin_op}}=`, then `{{bin_op}}|{{unary_op}}`, at `p`: where it ends.
    fn operator(&self, p: usize) -> Option<usize> {
        let c = self.ch(p);
        if p >= self.n || !b"+-*/%|&^<>=!?~".contains(&c) {
            return None;
        }
        for op in &BIN_OP {
            if self.lit(p, op) && self.ch(p + op.len()) == b'=' && p + op.len() < self.n {
                return Some(p + op.len() + 1);
            }
        }
        BIN_OP
            .iter()
            .chain(UNARY_OP.iter())
            .find(|op| self.lit(p, op))
            .map(|op| p + op.len())
    }

    /// `line_of_code_in_no_semicolon`'s patterns that start with a name: a generic call, a
    /// generic type's member, a call, an indexer.
    fn names_in_code(&mut self, p: usize) -> Option<usize> {
        let c = self.ch(p);
        if !(c.is_ascii_alphabetic() || c == b'_' || c == b'@' || c == b'\\' || c >= 0x80) {
            return None;
        }
        // `({{name}})(<)(?=[^(]*>\s*\()`
        let mut hit = None;
        self.names(p, &mut |e| {
            if self.ch(e) == b'<' && e < self.n && self.generic_call_paren_ahead(e + 1) {
                hit = Some(e);
                return true;
            }
            false
        });
        if let Some(e) = hit {
            self.put(p, e, VAR_FUNCTION);
            return self.tok_push(e, e + 1, GENERIC_BEGIN, &[N_GENERIC_CALL]);
        }
        // `({{name}})(<)(?={{namespaced_name}}>\s*\.)`
        let mut hit = None;
        self.names(p, &mut |e| {
            if self.ch(e) == b'<'
                && e < self.n
                && self.namespaced(e + 1, &mut |g| {
                    self.ch(g) == b'>' && {
                        let s = self.sp(g + 1);
                        self.ch(s) == b'.' && s < self.n
                    }
                })
            {
                hit = Some(e);
                return true;
            }
            false
        });
        if let Some(e) = hit {
            self.put(p, e, SUPPORT_TYPE);
            return self.tok_push(e, e + 1, GENERIC_BEGIN, &[GEN_TYPES]);
        }
        // `((?!{{reserved}}\b){{name}})\s*(\()`
        if self.reserved_word(p).is_none()
            && let Some((e, s)) = self.name_then(p, b'(')
        {
            self.put(p, e, FC_FUNCTION);
            self.put(e, s, FUNCTION_CALL);
            return self.tok_push(s, s + 1, FC_GROUP_BEGIN, &[ARGUMENTS]);
        }
        // `({{name}})\s*(\??\[)`
        let mut hit = None;
        self.names(p, &mut |e| {
            let s = self.sp(e);
            if s < self.n
                && (self.ch(s) == b'['
                    || (self.ch(s) == b'?' && self.ch(s + 1) == b'[' && s + 1 < self.n))
            {
                hit = Some((e, s));
                return true;
            }
            false
        });
        if let Some((e, s)) = hit {
            let end = if self.ch(s) == b'[' { s + 1 } else { s + 2 };
            self.put(p, e, VAR_OTHER);
            return self.tok_push(s, end, M_BRACKETS_BEGIN, &[ACCESSOR_ARGS]);
        }
        None
    }

    /// `(?=[^(]*>\s*\()` at `i`.
    fn generic_call_paren_ahead(&self, i: usize) -> bool {
        let mut r = i;
        while self.on(r) && self.ch(r) != b'(' {
            r += 1;
        }
        if r >= self.n {
            return false;
        }
        let mut t = r;
        while t > i && self.space(t - 1) {
            t -= 1;
        }
        t > i && self.ch(t - 1) == b'>'
    }

    /// `line_of_code_in_no_semicolon`'s `(` patterns.
    fn paren(&mut self, p: usize) -> Option<usize> {
        // `(\()\s*(\*)\s*({{name}})\s*(\))`
        let s = self.sp(p + 1);
        if self.ch(s) == b'*' && s < self.n {
            let t = self.sp(s + 1);
            let mut hit = None;
            self.names(t, &mut |e| {
                let u = self.sp(e);
                if self.ch(u) == b')' && u < self.n {
                    hit = Some((e, u));
                    return true;
                }
                false
            });
            if let Some((e, u)) = hit {
                self.put(p, p + 1, GROUP_BEGIN);
                self.put(s, s + 1, POINTER);
                self.put(t, e, VAR_OTHER);
                return self.tok(u, u + 1, GROUP_END);
            }
        }
        // `(\()(?={{namespaced_name}}\s*\))`
        if self.namespaced(p + 1, &mut |g| {
            let s = self.sp(g);
            self.ch(s) == b')' && s < self.n
        }) {
            return self.tok_push(p, p + 1, GROUP_BEGIN, &[CAST]);
        }
        // `\((?=(?:[^,)(]|\([^\)]*\))*,)`
        if self.tuple_ahead(p + 1) {
            return self.tok_push(p, p + 1, GROUP_BEGIN, &[TUPLE]);
        }
        self.tok_push(p, p + 1, GROUP_BEGIN, &[GROUP])
    }

    /// `(?=(?:[^,)(]|\([^\)]*\))*,)` at `i`.
    fn tuple_ahead(&self, mut i: usize) -> bool {
        loop {
            if !self.on(i) {
                return false;
            }
            match self.ch(i) {
                b',' => return i < self.n,
                b')' => return false,
                b'(' => {
                    let mut j = i + 1;
                    while self.on(j) && self.ch(j) != b')' {
                        j += 1;
                    }
                    if j >= self.n {
                        return false;
                    }
                    i = j + 1;
                }
                _ => i += 1,
            }
        }
    }

    /// `literals`.
    fn literals(&mut self, p: usize) -> Option<usize> {
        let c = self.ch(p);
        if p >= self.n {
            return None;
        }
        match c {
            b't' | b'f' | b'n' => {
                if let Some(e) = self.kw_any(p, &[b"true", b"false", b"null"]) {
                    return self.tok(p, e, LANGUAGE);
                }
                None
            }
            b'\'' => self.char_literal(p),
            b'0'..=b'9' => self.number(p),
            b'"' => {
                let q = self.open(p, p + 1, STRING_BEGIN, &[STRING]);
                self.regex_in(p);
                q
            }
            // `{{dec_digits}}` can start with `_`, and `\d` is Unicode's: a float only
            b'_' => self.float(p),
            0x80.. if self.digit(p).is_some() => self.float(p),
            _ => None,
        }
    }

    /// The character literals.
    fn char_literal(&mut self, p: usize) -> Option<usize> {
        if self.ch(p + 1) == b'\'' && p + 1 < self.n {
            return self.tok(p, p + 2, LONE_ESCAPE);
        }
        if let Some(e) = self.escaped_char_then_quote(p + 1) {
            self.put(p, p + 1, CHAR_LEAF);
            self.put(p + 1, e, CHAR_ESCAPE);
            return self.tok(e, e + 1, CHAR_LEAF);
        }
        if p + 1 < self.n {
            let e = self.next(p + 1);
            if self.ch(e) == b'\'' && e < self.n {
                return self.tok(p, e + 1, CHAR_LEAF);
            }
        }
        let mut e = p + 1;
        while e < self.n && self.b[e] != b'\'' {
            e += 1;
        }
        if e < self.n && e > p + 1 {
            return self.tok(p, e + 1, NOT_A_CHAR);
        }
        None
    }

    /// The number patterns.
    fn number(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        // `(0[xX])[\h_]*\h`, `(0[bB])[01_]*[01]`
        if b[p] == b'0' && p + 1 < self.n {
            let (digit, hex): (fn(u8) -> bool, bool) = match b[p + 1] {
                b'x' | b'X' => (|c: u8| c.is_ascii_hexdigit(), true),
                b'b' | b'B' => (|c: u8| c == b'0' || c == b'1', false),
                _ => (|_| false, false),
            };
            let mut e = p + 2;
            let mut last = None;
            while e < self.n && (digit(b[e]) || b[e] == b'_') {
                if digit(b[e]) {
                    last = Some(e + 1);
                }
                e += 1;
            }
            if let Some(end) = last {
                let (base, leaf) = if hex {
                    (HEX_BASE, HEX)
                } else {
                    (BIN_BASE, BIN)
                };
                self.put(p, p + 2, base);
                return self.tok(p + 2, end, leaf);
            }
        }
        if let Some(q) = self.float(p) {
            return Some(q);
        }
        // `(?:0|[1-9]{{dec_digits}}?)({{integer_suffix}})?`
        let mut e = p + 1;
        if b[p] != b'0' {
            self.dec_digits(p + 1, &mut |x| {
                e = x;
                true
            });
        }
        self.put(p, e, DECIMAL);
        let s = match (self.ch(e), self.ch(e + 1)) {
            (b'u' | b'U', b'l' | b'L') if e + 1 < self.n => 2,
            (b'l' | b'L', b'u' | b'U') if e + 1 < self.n => 2,
            (b'u' | b'U' | b'l' | b'L', _) if e < self.n => 1,
            _ => 0,
        };
        if s > 0 {
            return self.tok(e, e + s, DEC_SUFFIX);
        }
        self.fin(e)
    }

    /// `{{dec_digits}}(?:(?:(?:(\.){{dec_digits}}){{dec_exponent}}?|{{dec_exponent}})({{float_suffix}})?|({{float_suffix}}))`.
    fn float(&mut self, p: usize) -> Option<usize> {
        let mut first = None;
        self.dec_digits(p, &mut |e| {
            first = Some(e);
            true
        });
        if let Some(e) = first {
            let suffix = |s: &Self, i: usize| {
                i < s.n && matches!(s.b[i], b'f' | b'F' | b'd' | b'D' | b'm' | b'M')
            };
            let exponent = |s: &Self, i: usize| -> Option<usize> {
                if i >= s.n || !matches!(s.b[i], b'e' | b'E') {
                    return None;
                }
                let mut end = None;
                s.dec_digits(i + 1, &mut |x| {
                    end = Some(x);
                    true
                });
                if end.is_none() && i + 1 < s.n && matches!(s.b[i + 1], b'-' | b'+') {
                    s.dec_digits(i + 2, &mut |x| {
                        end = Some(x);
                        true
                    });
                }
                end
            };
            let mut float = None;
            if self.ch(e) == b'.' && e < self.n {
                let mut f = None;
                self.dec_digits(e + 1, &mut |x| {
                    f = Some(x);
                    true
                });
                if let Some(f) = f {
                    let g = exponent(self, f).unwrap_or(f);
                    float = Some((Some(e), g));
                }
            }
            if float.is_none()
                && let Some(g) = exponent(self, e)
            {
                float = Some((None, g));
            }
            if let Some((dot, g)) = float {
                if let Some(d) = dot {
                    self.put(p, d, FLOAT);
                    self.put(d, d + 1, FLOAT_DOT);
                }
                self.put(p, g, FLOAT);
                if suffix(self, g) {
                    return self.tok(g, g + 1, FLOAT_SUFFIX);
                }
                return self.fin(g);
            }
            if suffix(self, e) {
                self.put(p, e, FLOAT);
                return self.tok(e, e + 1, FLOAT_SUFFIX);
            }
        }
        None
    }

    /// The match ends at `e`, its text the context's.
    fn fin(&mut self, e: usize) -> Option<usize> {
        self.fill(e);
        Some(e)
    }

    /// `type`.
    fn type_(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.type_common(p) {
            return Some(q);
        }
        if self.ch(p) == b'(' && p < self.n {
            return self.tok_push(p, p + 1, GROUP_BEGIN, &[TUPLE_T]);
        }
        None
    }

    /// `type_common`.
    fn type_common(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.namespace_alias(p, NAMESPACE_VAR) {
            return Some(q);
        }
        for w in [&b"class"[..], b"struct", b"enum"] {
            if self.lit(p, w) {
                return self.tok(p, p + w.len(), STORAGE_TYPE_OTHER);
            }
        }
        if let Some(e) = self.new_parens(p) {
            return self.tok(p, e, NEW_OP);
        }
        if let Some(e) = self.base_type(p) {
            return self.tok(p, e, STORAGE_TYPE);
        }
        if let Some(q) = self.brackets_capture(p) {
            return Some(q);
        }
        let s = self.sp(p);
        match self.ch(s) {
            b'.' if s < self.n => {
                self.put(s, s + 1, DOT_NAMESPACE);
                let t = self.sp(s + 1);
                return self.fin(t);
            }
            _ => {}
        }
        if self.ch(p) == b'*' && p < self.n {
            return self.tok(p, p + 1, POINTER);
        }
        match self.ch(s) {
            b'?' if s < self.n => return self.tok(s, s + 1, NULLABLE),
            b'<' if s < self.n => return self.tok_push(s, s + 1, GENERIC_BEGIN, &[TYPE_ARG]),
            _ => {}
        }
        if let Some(e) = self.name(p) {
            return self.tok(p, e, SUPPORT_TYPE);
        }
        let c = self.ch(p);
        if matches!(c, b'{' | b':') && p < self.n {
            return self.close(p, p + 1, ILLEGAL);
        }
        if (matches!(c, b'}' | b')' | b'>' | b']' | b',' | b';') && p < self.n)
            || self.lit(p, b"=>")
        {
            return self.leave(p);
        }
        None
    }

    /// `type_arg_param_common`.
    fn type_arg_param_common(&mut self, p: usize) -> Option<usize> {
        match self.ch(p) {
            b',' if p < self.n => return self.tok(p, p + 1, TYPE_SEP),
            b'>' => {
                self.put(p, p + 1, GENERIC_END);
                let x = self.type_suffix_capture(p + 1);
                self.suffix_tokens(&x, None);
                self.pop();
                return Some(x.end);
            }
            _ => {}
        }
        self.type_(p)
    }

    /// `type_parameter`.
    fn type_parameter(&mut self, p: usize) -> Option<usize> {
        for w in [&b"in"[..], b"out"] {
            if self.kw_end(p, w) {
                return self.tok(p, p + w.len(), MODIFIER);
            }
        }
        self.type_arg_param_common(p)
    }

    /// `preprocessor_option`.
    fn preproc(&mut self, p: usize) -> Option<usize> {
        if !self.word_before(p) {
            for w in [&b"define"[..], b"undef"] {
                if self.lit(p, w)
                    && let Some(s) = self.sp1(p + w.len())
                {
                    let mut hit = None;
                    self.names(s, &mut |e| {
                        if !self.word_at(e) || self.bnd(e) {
                            hit = Some(e);
                            return true;
                        }
                        false
                    });
                    if let Some(e) = hit {
                        self.put(p, p + w.len(), KW_CONTROL_PP);
                        return self.tok(s, e, ENTITY_CONSTANT);
                    }
                }
            }
            if let Some(e) = self.kw_any(p, &[b"if", b"elif"]) {
                return self.tok_push(p, e, KW_CONTROL_PP, &[PP_IF]);
            }
            if let Some(e) = self.kw_any(p, &[b"else", b"endif"]) {
                return self.tok_push(p, e, KW_CONTROL_PP, &[OPTION_DONE]);
            }
            for (w, leaf) in [
                (&b"error"[..], UNQUOTED),
                (b"warning", UNQUOTED),
                (b"region", SECTION),
                (b"endregion", SECTION_END),
            ] {
                if self.kw(p, w) {
                    let s = self.sp_line(p + w.len());
                    self.put(p, p + w.len(), KW_OTHER_PP);
                    return self.tok(s, self.n, leaf);
                }
            }
            if self.lit(p, b"line")
                && let Some(s) = self.sp1(p + 4)
            {
                if let Some(e) = self.kw_any(s, &[b"default", b"hidden"]) {
                    self.put(p, p + 4, KW_OTHER_PP);
                    return self.tok_push(s, e, KW_OTHER_PP, &[OPTION_DONE]);
                }
                let d = self.digits(s).unwrap_or(s);
                if let Some(t) = self.sp1(d) {
                    self.put(p, p + 4, KW_OTHER_PP);
                    self.put(s, d, DECIMAL);
                    let mut end = t;
                    if self.ch(t) == b'"' && t < self.n {
                        let last = (t + 1..self.n).rev().find(|&i| self.b[i] == b'"');
                        if let Some(l) = last {
                            self.put(t, t + 1, PP_LINE_STRING_BEGIN);
                            self.put(t + 1, l, PP_LINE_STRING);
                            self.put(l, l + 1, PP_LINE_STRING_END);
                            end = l + 1;
                        }
                    }
                    self.fill(end);
                    self.push(OPTION_DONE);
                    return Some(end);
                }
            }
            if self.lit(p, b"pragma")
                && let Some(s) = self.sp1(p + 6)
                && self.lit(s, b"checksum")
                && let Some(t) = self.sp1(s + 8)
            {
                self.put(p, p + 6, KW_OTHER_PP);
                self.put(s, s + 8, KW_OTHER_PP);
                self.fill(t);
                self.push(PP_CHECKSUM);
                return Some(t);
            }
        }
        if self.lit(p, b"pragma")
            && let Some(s) = self.sp1(p + 6)
            && self.kw_end(s, b"warning")
        {
            self.put(p, p + 6, KW_OTHER_PP);
            return self.tok_push(s, s + 7, KW_OTHER_PP, &[PP_WARNING]);
        }
        if p == self.step && p < self.n {
            return self.tok(p, self.n, INVALID);
        }
        if self.eol(p) {
            return self.leave(p);
        }
        None
    }

    /// `#anon_preprocessor_option_0`: an `#if`'s condition.
    fn pp_if(&mut self, p: usize) -> Option<usize> {
        match self.ch(p) {
            b'(' | b')' if p < self.n => return self.tok(p, p + 1, PARENS),
            b'!' if p < self.n => return self.tok(p, p + 1, LOGICAL),
            _ => {}
        }
        if self.lit(p, b"&&") || self.lit(p, b"||") {
            return self.tok(p, p + 2, LOGICAL);
        }
        if let Some(e) = self.kw_any(p, &[b"true", b"false"]) {
            return self.tok(p, e, LANGUAGE);
        }
        if let Some(e) = self.name(p) {
            return self.tok(p, e, FLAG);
        }
        self.option_done(p)
    }

    /// `option_done`.
    fn option_done(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.comments(p) {
            return Some(q);
        }
        if self.eol(p) {
            return self.leave(p);
        }
        if self.nonspace(p) {
            let e = self.next(p);
            return self.tok(p, e, INVALID);
        }
        None
    }

    /// `#anon_preprocessor_option_3`: `#pragma warning`'s arguments.
    fn pp_warning(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.kw_any(p, &[b"disable", b"restore"]) {
            self.put(p, e, KW_OTHER_PP);
            if let Some(s) = self.sp1(e) {
                let mut t = s;
                while t < self.n {
                    let c = self.b[t];
                    if c.is_ascii_alphabetic() || c == b'_' || c == b'-' {
                        t += 1;
                    } else if c >= 0x80 && self.word_at(t) && !self.b[t].is_ascii_digit() {
                        t = self.next(t);
                    } else {
                        break;
                    }
                }
                if t > s {
                    return self.tok(s, t, WARNING_NAME);
                }
            }
            return self.fin(e);
        }
        if let Some(q) = self.comments(p) {
            return Some(q);
        }
        if self.eol(p) {
            return self.leave(p);
        }
        None
    }

    /// `comments_in`, in a comment whose leaves for a to-do and a link are given.
    fn comments_in(&mut self, p: usize, todo: u16, link: u16) -> Option<usize> {
        if let Some(e) = self.todo(p) {
            return self.tok(p, e, todo);
        }
        if self.lit(p, b"/!\\") {
            return self.tok(p, p + 3, todo);
        }
        if let Some(e) = self.link(p) {
            return self.tok(p, e, link);
        }
        None
    }

    /// `\b(?i:todo|hack|xxx)\b` at `p`: where it ends.
    fn todo(&self, p: usize) -> Option<usize> {
        if self.word_before(p) {
            return None;
        }
        let at = |w: &[u8]| {
            self.b
                .get(p..p + w.len())
                .is_some_and(|x| x.eq_ignore_ascii_case(w))
        };
        let e = if at(b"todo") || at(b"hack") {
            p + 4
        } else if at(b"xxx") {
            p + 3
        } else {
            return None;
        };
        (!self.word_at(e)).then_some(e)
    }

    /// `\b(https?://\S*)\b` at `p`: where it ends.
    fn link(&self, p: usize) -> Option<usize> {
        if self.word_before(p) || !self.lit(p, b"http") {
            return None;
        }
        let q = if self.ch(p + 4) == b's' { p + 5 } else { p + 4 };
        if !self.lit(q, b"://") {
            return None;
        }
        let mut e = q + 3;
        while self.nonspace(e) {
            e = self.next(e);
        }
        // back to a `\b`
        while e > q + 3 && !self.bnd(e) {
            e = self.prev(e);
        }
        if self.bnd(e) {
            return Some(e);
        }
        // `http://` alone: `\b` after `p` within it
        (q..=q + 3).rev().find(|&x| x > p && self.bnd(x))
    }

    /// Where the character before `i` starts.
    fn prev(&self, i: usize) -> usize {
        let mut s = i - 1;
        while s > 0 && self.b[s] & 0xc0 == 0x80 {
            s -= 1;
        }
        s
    }

    /// `documentation`.
    fn doc(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b'<' && p < self.n {
            if self.ch(p + 1) == b'/' {
                let mut hit = None;
                self.names(p + 2, &mut |e| {
                    if self.ch(e) == b'>' && e < self.n {
                        hit = Some(e);
                        return true;
                    }
                    false
                });
                // `(<)({{name}})` first: no name right after `<` here
                if let Some(e) = hit {
                    self.put(p, p + 2, TAG_BEGIN);
                    self.put(p + 2, e, TAG_NAME_END);
                    return self.tok(e, e + 1, TAG_END);
                }
            } else if let Some(e) = self.name(p + 1) {
                self.put(p, p + 1, TAG_BEGIN);
                return self.tok_push(p + 1, e, TAG_NAME_BEGIN, &[DOC_TAG]);
            }
        }
        if p == 0 {
            let s = self.ind;
            if self.lit(s, b"///") {
                return self.tok(s, s + 3, DOC_PUNCT);
            }
            // `^\s*(?!///)`: its spaces are out of the doc comment
            let e = self.sp(0);
            if e == 0 {
                return self.leave(0);
            }
            self.pop();
            self.fill(e);
            return Some(e);
        }
        if let Some(q) = self.comments_in(p, TODO, LINK) {
            return Some(q);
        }
        // `[\w\s]+|.`
        if self.word_at(p) || self.space(p) {
            let mut e = p;
            while self.word_at(e) || self.space(e) {
                e = self.next(e);
            }
            return self.tok(p, e, DOC_TEXT);
        }
        if p < self.n {
            let e = self.next(p);
            return self.tok(p, e, DOC_TEXT);
        }
        None
    }

    /// `#anon_documentation_0`: a tag's attributes.
    fn doc_tag(&mut self, p: usize) -> Option<usize> {
        if let Some((e, s)) = self.name_then(p, b'=') {
            self.put(p, e, ATTRIBUTE_NAME);
            return self.tok(s, s + 1, ARG_VALUE_SEP);
        }
        if self.lit(p, b"/>") {
            return self.close(p, p + 2, TAG_END);
        }
        match self.ch(p) {
            b'>' if p < self.n => return self.close(p, p + 1, TAG_END),
            b'"' if p < self.n => {
                if let Some(e) = (p + 1..self.n).find(|&i| self.b[i] == b'"') {
                    return self.tok(p, e + 1, DQ);
                }
            }
            _ => {}
        }
        if self.eol(p) {
            return self.leave(p);
        }
        None
    }

    /// `string`.
    fn string(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.string_escaped(p) {
            return Some(q);
        }
        if self.ch(p) == b'"' && p < self.n {
            return self.close(p, p + 1, STRING_END);
        }
        if let Some(q) = self.string_placeholders(p) {
            return Some(q);
        }
        if let Some(d) = self.brace_digits(p) {
            self.fill(p);
            self.push(S_PH);
            self.put(p, p + 1, PH_BEGIN);
            return self.tok(p + 1, d, DECIMAL);
        }
        if p == self.n {
            return self.close(p, p + 1, UNCLOSED_STRING);
        }
        if p > self.n {
            return self.leave(p);
        }
        None
    }

    /// `string_escaped`.
    fn string_escaped(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.escaped_char(p) {
            return self.tok(p, e, ESCAPE);
        }
        if self.ch(p) == b'\\' && p < self.n {
            return self.tok(p, p + 1, LONE_ESCAPE);
        }
        None
    }

    /// `(\{)(\d+)` at `p`: where the digits end.
    fn brace_digits(&self, p: usize) -> Option<usize> {
        if self.ch(p) != b'{' || p >= self.n {
            return None;
        }
        self.digits(p + 1)
    }

    /// `string_placeholders`.
    fn string_placeholders(&mut self, p: usize) -> Option<usize> {
        if self.lit(p, b"{{") || self.lit(p, b"}}") {
            return self.tok(p, p + 2, ESCAPE);
        }
        if self.ch(p) != b'{' || p >= self.n {
            return None;
        }
        // `\{\h{8}-(?:\h{4}-){3}\h{12}\}`
        let guid = |s: &Self| -> bool {
            let mut i = p + 1;
            for (k, n) in [8, 4, 4, 4, 12].into_iter().enumerate() {
                for _ in 0..n {
                    if i >= s.n || !s.b[i].is_ascii_hexdigit() {
                        return false;
                    }
                    i += 1;
                }
                let want = if k == 4 { b'}' } else { b'-' };
                if s.ch(i) != want || i >= s.n {
                    return false;
                }
                i += 1;
            }
            true
        };
        if guid(self) {
            return self.tok(p, p + 38, GUID);
        }
        // `(\{)(\d+)(?=")`
        if let Some(d) = self.brace_digits(p)
            && self.ch(d) == b'"'
            && d < self.n
        {
            self.put(p, p + 1, PH_BEGIN_P);
            return self.tok(p + 1, d, PH_UNCLOSED_NUM);
        }
        None
    }

    /// `long_string`.
    fn long_string(&mut self, p: usize) -> Option<usize> {
        if self.lit(p, b"\"\"") {
            return self.tok(p, p + 2, ESCAPE);
        }
        if let Some(q) = self.string_placeholders(p) {
            return Some(q);
        }
        if let Some(d) = self.brace_digits(p) {
            self.fill(p);
            self.push(R_PH);
            self.put(p, p + 1, PH_BEGIN);
            return self.tok(p + 1, d, DECIMAL);
        }
        if self.ch(p) == b'"' && p < self.n {
            return self.close(p, p + 1, STRING_END);
        }
        None
    }

    /// `format_string`.
    fn format_string(&mut self, p: usize) -> Option<usize> {
        if self.ch(p) == b'"' && p < self.n {
            return self.close(p, p + 1, STRING_END);
        }
        if let Some(q) = self.string_escaped(p) {
            return Some(q);
        }
        if self.lit(p, b"{{") || self.lit(p, b"}}") {
            return self.tok(p, p + 2, ESCAPE);
        }
        if self.ch(p) == b'{' && p < self.n {
            return self.open(p, p + 1, INTERP_BEGIN, &[INTERP]);
        }
        if p == self.n {
            return self.close(p, p + 1, UNCLOSED_STRING);
        }
        if p > self.n {
            return self.leave(p);
        }
        None
    }

    /// `long_format_string`.
    fn long_format_string(&mut self, p: usize) -> Option<usize> {
        if self.lit(p, b"\"\"") {
            return self.tok(p, p + 2, ESCAPE);
        }
        if self.ch(p) == b'"' && p < self.n {
            return self.close(p, p + 1, STRING_END);
        }
        if self.lit(p, b"{{") || self.lit(p, b"}}") {
            return self.tok(p, p + 2, ESCAPE);
        }
        if self.ch(p) == b'{' && p < self.n {
            return self.open(p, p + 1, INTERP_BEGIN, &[LINTERP]);
        }
        None
    }

    /// `string_placeholder` (a verbatim string's, `long`).
    fn placeholder(&mut self, p: usize, long: bool) -> Option<usize> {
        // `inside_string_placeholder`
        if self.ch(p) == b'}' && p < self.n {
            self.put(p, p + 1, PH_END);
            let mut e = p + 1;
            if self.ch(e) == b'}' && e < self.n && self.ch(e + 1) != b'}' {
                self.put(e, e + 1, UNESCAPED_PH);
                e += 1;
            }
            self.fill(e);
            self.pop();
            return Some(e);
        }
        if self.ch(p) == b'"'
            && p < self.n
            && let Some(q) = self.leave(p)
        {
            return Some(q);
        }
        let fs = if long { FS_R } else { FS_S };
        if let Some(q) = self.placeholder_format(p, long, fs) {
            return Some(q);
        }
        // `[^"}]+`
        if self.on(p) && !matches!(self.ch(p), b'"' | b'}') {
            let mut e = p;
            while self.on(e) && !matches!(self.ch(e), b'"' | b'}') {
                e += 1;
            }
            return self.tok(p, e, UNEXPECTED_IN_PH);
        }
        None
    }

    /// `string_placeholder_format` (a verbatim string's, `long`), its format spec `fs`.
    fn placeholder_format(&mut self, p: usize, long: bool, fs: u8) -> Option<usize> {
        // `\s*(?:(,)\s*(-?\d+)\s*)?`, at the step's start only, and only taking something
        if p == self.step {
            let s = self.sp(p);
            let mut end = s;
            let mut parts = None;
            if self.ch(s) == b',' && s < self.n {
                let t = self.sp(s + 1);
                let mut d = t;
                if self.ch(d) == b'-' && d < self.n {
                    d += 1;
                }
                if let Some(e) = self.digits(d) {
                    d = e;
                    parts = Some((s, t, d));
                    end = self.sp(d);
                }
            }
            if end > p {
                if let Some((s, t, d)) = parts {
                    self.put(s, s + 1, ARGS_SEP);
                    self.put(t, d, FORMATTING);
                }
                return self.fin(end);
            }
        }
        if self.ch(p) == b':' && p < self.n {
            let unclosed = if long {
                self.ch(p + 1) == b'"' && p + 1 < self.n && self.ch(p + 2) != b'"'
            } else {
                self.ch(p + 1) == b'"' && p + 1 < self.n
            };
            if unclosed {
                return self.close(p, p + 1, UNCLOSED_PH);
            }
            return self.open(p, p + 1, SEPARATOR, &[fs]);
        }
        None
    }

    /// `#anon_string_placeholder_format_0`: a format spec in a string.
    fn format_spec(&mut self, p: usize) -> Option<usize> {
        if self.lit(p, b"{{") || self.lit(p, b"}}") {
            return self.tok(p, p + 2, ESCAPE);
        }
        if let Some(q) = self.string_escaped(p) {
            return Some(q);
        }
        if self.ch(p) == b'}'
            && p < self.n
            && let Some(q) = self.leave(p)
        {
            return Some(q);
        }
        // `([^}"\\]+(\\.)*)+(?=")`
        let plain = |s: &Self, i: usize| s.on(i) && !matches!(s.ch(i), b'}' | b'"' | b'\\');
        if plain(self, p) {
            let mut e = p;
            loop {
                while plain(self, e) {
                    e += 1;
                }
                let mut esc = false;
                while self.ch(e) == b'\\' && e + 1 < self.n {
                    e = self.next(e + 1);
                    esc = true;
                }
                if !(esc && plain(self, e)) {
                    break;
                }
            }
            if self.ch(e) == b'"' && e < self.n {
                return self.close(p, e, UNCLOSED_PH);
            }
        }
        if self.ch(p) == b'{' && p < self.n {
            return self.tok(p, p + 1, UNESCAPED_PH);
        }
        None
    }

    /// `#anon_long_string_placeholder_format_0`: a format spec in a verbatim string.
    fn long_format_spec(&mut self, p: usize) -> Option<usize> {
        if self.lit(p, b"{{") || self.lit(p, b"}}") || self.lit(p, b"\"\"") {
            return self.tok(p, p + 2, ESCAPE);
        }
        if self.ch(p) == b'}'
            && p < self.n
            && let Some(q) = self.leave(p)
        {
            return Some(q);
        }
        // `\\(?:""|[^"])`
        if self.ch(p) == b'\\' && p < self.n {
            if self.lit(p + 1, b"\"\"") {
                return self.tok(p, p + 3, ESCAPE);
            }
            if self.on(p + 1) && self.ch(p + 1) != b'"' {
                let e = self.next(p + 1);
                return self.tok(p, e, ESCAPE);
            }
        }
        // `(?:[^}"]+|"")+(?="(?!"))`
        let mut e = p;
        loop {
            if self.on(e) && !matches!(self.ch(e), b'}' | b'"') {
                e += 1;
            } else if self.lit(e, b"\"\"") {
                e += 2;
            } else {
                break;
            }
        }
        if e > p && self.ch(e) == b'"' && e < self.n {
            return self.close(p, e, UNCLOSED_PH);
        }
        if self.ch(p) == b'{' && p < self.n {
            return self.tok(p, p + 1, UNESCAPED_PH);
        }
        None
    }
}
