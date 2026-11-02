//! The words bat's Lisp grammar lists, each pattern's (`(?i)`, ending at `{{break}}`), and the
//! one table that names, for a word, every list it is in: what it is at a list's head, in the
//! `identifier` context, in `loop`'s body, after `(declare (`, and whether it is a builtin
//! function or an operator. A token is looked up once, its whole text, ASCII case ignored.
//!
//! The lists are bat's patterns expanded (`bench/b2/root/build/lisp-scan/words.py` over the
//! probe's dump of the grammar); only the table is in the binary: the lists are consts, read
//! where the table is built.

/// What a word is at a list's head (`paren-begin`), in the grammar's order: the first list it
/// is in. 0 for none.
pub(super) const H_CLASS: u32 = 1;
pub(super) const H_LAMBDA: u32 = 2;
/// `def…un`, `defgeneric`, `define-command`: a function's name, then its parameters.
pub(super) const H_DEFUN: u32 = 3;
pub(super) const H_METHOD: u32 = 4;
pub(super) const H_FLET: u32 = 5;
pub(super) const H_MACRO: u32 = 6;
pub(super) const H_MACROLET: u32 = 7;
pub(super) const H_STRUCT: u32 = 8;
pub(super) const H_TYPE: u32 = 9;
pub(super) const H_LET: u32 = 10;
pub(super) const H_CONSTANT: u32 = 11;
pub(super) const H_VAR: u32 = 12;
pub(super) const H_DECLARATION: u32 = 13;
pub(super) const H_DECLARE: u32 = 14;
pub(super) const H_CONDITIONAL: u32 = 15;
pub(super) const H_RETURN: u32 = 16;
pub(super) const H_DOLIST: u32 = 17;
pub(super) const H_DO: u32 = 18;
pub(super) const H_CONTROL: u32 = 19;
pub(super) const H_LOOP: u32 = 20;
pub(super) const H_COMPARISON: u32 = 21;
pub(super) const H_ARITHMETIC: u32 = 22;
pub(super) const H_LOGICAL: u32 = 23;
pub(super) const H_BITWISE: u32 = 24;
pub(super) const H_QUOTE: u32 = 25;
const HEAD: u32 = 0x1f;

/// What a word is in the `identifier` context, in its order. 0 for none.
pub(super) const I_TYPE: u32 = 1 << 5;
pub(super) const I_BOOLEAN: u32 = 2 << 5;
pub(super) const I_NULL: u32 = 3 << 5;
pub(super) const I_CONSTANT: u32 = 4 << 5;
pub(super) const I_VARIABLE: u32 = 5 << 5;
const IDENT: u32 = 7 << 5;

/// A storage type or a constant: not a function at a list's head (`other-function`'s
/// lookahead), but what the `identifier` context makes it.
pub(super) const NOT_FUNCTION: u32 = 1 << 8;

/// What a word is in `loop`'s body, in its order. 0 for none.
pub(super) const L_NAMED: u32 = 1 << 9;
pub(super) const L_WITH: u32 = 2 << 9;
pub(super) const L_CONTROL: u32 = 3 << 9;
pub(super) const L_CONDITIONAL: u32 = 4 << 9;
pub(super) const L_FLOW: u32 = 5 << 9;
pub(super) const L_RETURN: u32 = 6 << 9;
pub(super) const L_OPERATOR: u32 = 7 << 9;
pub(super) const L_INTO: u32 = 8 << 9;
pub(super) const L_AND: u32 = 9 << 9;
pub(super) const L_OTHER: u32 = 10 << 9;
pub(super) const LOOP_WORD: u32 = 15 << 9;

/// A word `(declare (` takes: a storage modifier.
pub(super) const DECLARE_ID: u32 = 1 << 13;
/// One of the builtin functions.
pub(super) const BUILTIN: u32 = 1 << 14;

/// What an operator a word is (`operator`, after `#'` too). 0 for none.
pub(super) const O_COMPARISON: u32 = 1 << 15;
pub(super) const O_ARITHMETIC: u32 = 2 << 15;
pub(super) const O_LOGICAL: u32 = 3 << 15;
pub(super) const O_BITWISE: u32 = 4 << 15;
pub(super) const OPERATOR: u32 = 7 << 15;

/// What a word is at a list's head in Scheme and Racket ([`super::Dialect::Scheme`]): a
/// design of ada's own, not bat's (whose `Lisp` knows Common Lisp's heads, and whose `Racket`
/// a handful of words). 0 for none.
pub(super) const S_DEFINE: u32 = 1 << 18;
pub(super) const S_DEFINE_SYNTAX: u32 = 2 << 18;
pub(super) const S_DEFINE_VALUES: u32 = 3 << 18;
pub(super) const S_RECORD: u32 = 4 << 18;
pub(super) const S_LAMBDA: u32 = 5 << 18;
pub(super) const S_CASE_LAMBDA: u32 = 6 << 18;
pub(super) const S_LET: u32 = 7 << 18;
pub(super) const S_DO: u32 = 8 << 18;
pub(super) const S_CONDITIONAL: u32 = 9 << 18;
pub(super) const S_LOOP: u32 = 10 << 18;
pub(super) const S_CONTROL: u32 = 11 << 18;
pub(super) const S_LOGICAL: u32 = 12 << 18;
pub(super) const S_MODULE: u32 = 13 << 18;
pub(super) const S_QUOTE: u32 = 14 << 18;
pub(super) const S_ARITHMETIC: u32 = 15 << 18;
pub(super) const S_COMPARISON: u32 = 16 << 18;
const SCHEME_HEAD: u32 = 31 << 18;

/// A Scheme or Racket procedure of the standard libraries (R7RS small's, and Racket's most used).
pub(super) const S_BUILTIN: u32 = 1 << 23;

/// What a word is outside a head in Scheme and Racket. 0 for none.
pub(super) const S_CONSTANT: u32 = 1 << 24;
/// `else` and `=>`.
pub(super) const S_KEYWORD: u32 = 2 << 24;
const SCHEME_IDENT: u32 = 3 << 24;

/// What a word is at a list's head in Clojure ([`super::clojure`]): a design of ada's own, not
/// bat's (whose `Clojure` colors the `def` family and leaves every other head a plain call).
/// 0 for none. Clojure is case-sensitive and its core is lowercase: [`clojure_lookup`].
pub(super) const C_CONDITIONAL: u32 = 1 << 26;
pub(super) const C_CONTROL: u32 = 2 << 26;
pub(super) const C_LOOP: u32 = 3 << 26;
pub(super) const C_LOGICAL: u32 = 4 << 26;
pub(super) const C_MODULE: u32 = 5 << 26;
/// Arithmetic, comparison and the threading macros.
pub(super) const C_OPERATOR: u32 = 6 << 26;
/// The binding forms: `let`, `binding`, `with-open`, ….
pub(super) const C_BINDING: u32 = 7 << 26;
const CLOJURE_HEAD: u32 = 7 << 26;
/// A function of `clojure.core`, the most used.
pub(super) const C_BUILTIN: u32 = 1 << 29;

/// bat's `class-declaration`, pattern 1: 2 words.
const W_CLASS: &str = "defclass define-class";
/// bat's `function-declaration`, pattern 1: 1 words.
const W_LAMBDA: &str = "lambda";
/// bat's `function-declaration`, pattern 3: 1 words.
const W_METHOD: &str = "defmethod";
/// bat's `function-declaration`, pattern 4: 2 words.
const W_FLET: &str = "flet labels";
/// bat's `macro-declaration`, pattern 1: 1 words.
const W_MACRO: &str = "defmacro";
/// bat's `macro-declaration`, pattern 2: 1 words.
const W_MACROLET: &str = "macrolet";
/// bat's `struct-declaration`, pattern 1: 2 words.
const W_STRUCT: &str = "define-struct defstruct";
/// bat's `type-declaration`, pattern 1: 2 words.
const W_TYPE: &str = "define-type deftype";
/// bat's `variable-declaration`, pattern 1: 2 words.
const W_LET: &str = "let let*";
/// bat's `variable-declaration`, pattern 2: 1 words.
const W_CONSTANT: &str = "defconstant";
/// bat's `variable-declaration`, pattern 3: 2 words.
const W_VAR: &str = "defparameter defvar";
/// bat's `other-declaration`, pattern 1: 40 words.
const W_DECLARATION: &str = "declaim defclass defconstant defgeneric define-compiler-macro define-condition \
    define-declaration define-method-combination define-modify-macro define-setf-method \
    defmacro defmethod defpackage defparameter defsetf defstruct deftype defun defvar flet \
    generic-flet generic-function generic-labels labels let let* locally macrolet \
    multiple-value-bind proclaim prog prog* with-accessors with-added-methods \
    with-condition-restarts with-input-from-string with-open-file with-open-stream \
    with-output-to-string with-standard-io-syntax";
/// bat's `declare`, pattern 1: 1 words.
const W_DECLARE: &str = "declare";
/// bat's `control`, pattern 1: 9 words.
const W_CONDITIONAL: &str = "case cond ctypecase etypecase if otherwise typecase unless when";
/// bat's `control`, pattern 2: 2 words.
const W_RETURN: &str = "return return-from";
/// bat's `control`, pattern 3: 2 words.
const W_DOLIST: &str = "dolist dotimes";
/// bat's `control`, pattern 4: 6 words.
const W_DO: &str = "do do* do-all-symbols do-external-symbols do-symbols iterate";
/// bat's `control`, pattern 5: 7 words.
const W_CONTROL: &str = "block mapc mapcan mapcar mapcon mapl maplist";
/// bat's `loop`, pattern 1: 1 words.
const W_LOOP: &str = "loop";
/// bat's `operator`, pattern 1: 13 words.
const W_COMPARISON: &str = "/= < <= = > >= eq eql equal equalp max min neq";
/// bat's `operator`, pattern 3: 8 words.
const W_ARITHMETIC: &str = "* + - / decf incf mod rem";
/// bat's `operator`, pattern 4: 4 words.
const W_LOGICAL: &str = "and not or satisfies";
/// bat's `operator`, pattern 5: 5 words.
const W_BITWISE: &str = "logand logeqv logior lognor logxor";
/// bat's `quote`, pattern 1: 1 words.
const W_QUOTE: &str = "quote";
/// bat's `builtin-function`, pattern 1: 721 words.
const W_BUILTIN: &str = "abort abs acons acos acosh add-method adjoin adjust-array adjustable-array-p alpha-char-p \
    alphanumericp alter append appending apply applyhook apropos apropos-list aref \
    arithmetic-error-operands arithmetic-error-operation array-dimension array-dimensions \
    array-element-type array-has-fill-pointer-p array-in-bounds-p array-rank \
    array-row-major-index array-total-size arrayp ash asin asinh assert assoc assoc-if \
    assoc-if-not atan atanh atom augment-environment bit bit-and bit-andc1 bit-andc2 bit-eqv \
    bit-ior bit-nand bit-nor bit-not bit-orc1 bit-orc2 bit-vector-p bit-xor boole both-case-p \
    boundp break broadcast-stream-streams butlast byte byte-position byte-size caaaar caaadr \
    caaar caadar caaddr caadr caar cadaar cadadr cadar caddar cadddr caddr cadr call-method \
    call-next-method car catenate ccase cdaaar cdaadr cdaar cdadar cdaddr cdadr cdar cddaar \
    cddadr cddar cdddar cddddr cdddr cddr cdr ceiling cell-error-name cerror change-class \
    char char-bit char-bits char-code char-downcase char-equal char-font char-greaterp \
    char-int char-lessp char-name char-not-equal char-not-greaterp char-not-lessp char-upcase \
    char/= char< char<= char= char> char>= character characterp check-type choose choose-if \
    chunk cis class-name class-of clear-input close clrhash code-char coerce collect \
    collect-alist collect-and collect-append collect-file collect-first collect-fn \
    collect-hash collect-last collect-length collect-max collect-min collect-nconc \
    collect-nth collect-or collect-plist collect-sum collecting collecting-fn commonp compile \
    compile-file compile-file-pathname compiled-function-p compiler-let \
    compiler-macro-function compiler-macroexpand compiler-macroexpand-1 complement complex \
    complexp compute-applicable-methods compute-restarts concatenate \
    concatenated-stream-streams conjugate cons consp constantp continue copy-alist copy-list \
    copy-pprint-dispatch copy-readtable copy-seq copy-symbol copy-tree cos cosh cotruncate \
    count count-if count-if-not counting declaim declaration-information decode-float \
    decode-universal-time delete delete-duplicates delete-file delete-if delete-if-not \
    delete-package denominator deposit-field describe describe-object destructuring-bind \
    digit-char digit-char-p directory directory-namestring disassemble do-all-keyword-symbols \
    do-external-keyword-symbols do-keyword-symbols documentation dolist dpb dribble ecase \
    echo-stream-input-stream echo-stream-output-stream ed eighth elt encapsulated enclose \
    encode-universal-time endp enough-namestring ensure-generic-function error eval evalhook \
    evenp every exp expand export expt f fboundp fdefinition ffloor fifth file-author \
    file-error-pathname file-length file-namestring file-position file-string-length \
    file-write-date fill fill-pointer find find-all-keyword-symbols find-class find-if \
    find-if-not find-method find-package find-restart find-symbol finish-output first float \
    float-digits float-precision float-radix float-sign floatp floor format formatter fourth \
    funcall function-information function-keywords function-lambda-expression functionp \
    gatherer gathering gcd generator generic-function gensym gentemp get get-decoded-time \
    get-internal-real-time get-internal-run-time get-output-stream-string get-properties \
    get-setf-method get-setf-method-multiple-value get-universal-time getf gethash \
    graphic-char-p handler-bind handler-case hash-table-count hash-table-p \
    hash-table-rehash-size hash-table-rehash-threshold hash-table-size hash-table-test \
    host-namestring identity ignore-errors imagpart import in-package initialize-instance \
    input-stream-p inspect int-char integer-decode-float integer-length integerp \
    interactive-stream-p intern intersection invalid-method-error invoke-debugger \
    invoke-restart isqrt iterate keywordp last latch lcm ldb ldb-test ldiff length \
    lisp-implementation-type lisp-implementation-version list list* list-all-packages \
    list-length listen listp load load-logical-pathname-translations locally log logandc1 \
    logandc2 logbitp logcount logical-pathname logical-pathname-translations lognand lognot \
    logorc1 logorc2 logtest long-site-name loop-finish lower-case-p machine-instance \
    machine-type machine-version macro-function macroexpand macroexpand-1 make-array \
    make-broadcast-stream make-char make-concatenated-stream make-condition \
    make-dispatch-macro-character make-echo-stream make-hash-table make-instance \
    make-instances-obsolete make-list make-load-form make-load-form-saving-slots make-package \
    make-pathname make-random-state make-sequence make-string make-string-input-stream \
    make-string-output-stream make-symbol make-synonym-stream make-two-way-stream makunbound \
    map map-fn map-into maphash mapping mask mask-field maximize maximizing member member-if \
    member-if-not memberp merge merge-pathnames method-combination-error method-qualifiers \
    mingle minimize minimizing minusp mismatch muffle-warning multiple-value-bind \
    multiple-value-list multiple-value-setq name-char namestring nbutlast nconc nconcing \
    next-in next-method-p next-out nintersection ninth no-applicable-method no-next-method \
    notany notevery nreconc nreverse nset-difference nset-exclusive-or nstring-capitalize \
    nstring-downcase nstring-upcase nsublis nsubst nsubst-if nsubst-if-not nsubstitute \
    nsubstitute-if nsubstitute-if-not nth nth-value nthcdr numberp numerator nunion oddp open \
    open-stream-p output-stream-p package-error-package package-name package-nicknames \
    package-shadowing-keyword-symbols package-use-list package-used-by-list packagep pairlis \
    parse-integer parse-macro parse-namestring pathname pathname-device pathname-directory \
    pathname-host pathname-match-p pathname-name pathname-type pathname-version pathnamep \
    peek-char phase plusp pop position position-if position-if-not positions pprint-dispatch \
    pprint-exit-if-list-exhausted pprint-fill pprint-indent pprint-linear \
    pprint-logical-block pprint-newline pprint-pop pprint-tab pprint-tabular previous prin1 \
    print print-object print-unreadable-object probe-file producing prog1 prog2 progn provide \
    psetf psetq push pushnew quote random random-state-p rassoc rassoc-if rassoc-if-not \
    rational rationalize rationalp read read-byte read-char read-char-no-hang \
    read-delimited-list read-from-string read-line read-preserving-whitespace readtable-case \
    readtablep realp realpart reduce reinitialize-instance remf remhash remove \
    remove-duplicates remove-method remprop rename-file rename-package replace require rest \
    restart-bind restart-case restart-name result-of revappend reverse room rotatef round \
    row-major-aref rplaca rplacd sbit scale-float scan scan-alist scan-file scan-fn \
    scan-fn-inclusive scan-hash scan-lists-of-lists scan-lists-of-lists-fringe scan-multiple \
    scan-plist scan-range scan-sublists scan-symbols schar search second series set \
    set-char-bit set-difference set-dispatch-macro-character set-exclusive-or \
    set-macro-character set-pprint-dispatch set-syntax-from-char setf setq seventh shadow \
    shadowing-import shared-initialize shiftf short-site-name signal signum \
    simple-bit-vector-p simple-condition-format-arguments simple-condition-format-string \
    simple-string-p simple-vector-p sin sinh sixth sleep slot-boundp slot-exists-p \
    slot-makunbound slot-missing slot-unbound slot-value software-type software-version some \
    sort special-form-p split split-if sqrt stable-sort standard-char-p step store-value \
    stream-element-type stream-error-stream stream-external-format streamp string \
    string-capitalize string-downcase string-equal string-greaterp string-left-trim \
    string-lessp string-not-equal string-not-greaterp string-not-lessp string-right-trim \
    string-trim string-upcase string/= string< string<= string= string> string>= stringp \
    sublis subseq subseries subsetp subst subst-if subst-if-not substitute substitute-if \
    substitute-if-not subtypep sum summing svref sxhash symbol-function symbol-name \
    symbol-package symbol-plist symbol-value symbolp synonym-stream-symbol tailp tan tanh \
    tenth terminate-producing terpri third time to-alter trace translate-logical-pathname \
    translate-pathname tree-equal truename truncate two-way-stream-input-stream \
    two-way-stream-output-stream type-error-datum type-error-expected-type type-of typep \
    unexport unintern union unread-char untrace unuse-package \
    update-instance-for-different-class update-instance-for-redefined-class \
    upgraded-array-element-type upgraded-complex-part-type upper-case-p use-package use-value \
    user-homedir-pathname values values-list variable-information vector vector-pop \
    vector-push vector-push-extend vectorp warn wild-pathname-p with-compilation-unit \
    with-hash-table-iterator with-package-iterator with-simple-restart \
    with-standard-io-syntax write write-byte write-char write-string write-to-string y-or-n-p \
    yes-or-no-p zerop";
/// bat's `identifier`, pattern 1: 94 words.
const W_STORAGE_TYPE: &str = "arithmetic-error array atom base-char base-string bignum bit bit-vector broadcast-stream \
    built-in-class cell-error character class compiled-function complex concatenated-stream \
    condition cons control-error division-by-zero double-float echo-stream end-of-file error \
    extended-char file-error file-stream fixnum float floating-point-inexact \
    floating-point-invalid-operation floating-point-overflow floating-point-underflow \
    function generic-function hash-table integer keyword list logical-pathname long-float \
    method method-combination number package package-error parse-error pathname \
    print-not-readable program-error random-state ratio rational reader-error readtable real \
    restart sequence serious-condition short-float signed-byte simple-array \
    simple-base-string simple-bit-vector simple-condition simple-error simple-string \
    simple-type-error simple-vector simple-warning single-float standard-char standard-class \
    standard-generic-function standard-method standard-object storage-condition stream \
    stream-error string string-stream structure-class structure-object style-warning symbol \
    synonym-stream two-way-stream type-error unbound-slot unbound-variable undefined-function \
    unsigned-byte vector warning";
/// bat's `identifier`, pattern 2: 2 words.
const W_BOOLEAN: &str = "false true";
/// bat's `identifier`, pattern 3: 2 words.
const W_NULL: &str = "nil null";
/// bat's `identifier`, pattern 4: 55 words.
const W_CONSTANT_LANGUAGE: &str = "array-dimension-limit array-rank-limit array-total-size-limit call-arguments-limit \
    char-bits-limit char-code-limit char-control-bit char-font-limit char-hyper-bit \
    char-meta-bit char-super-bit double-float-epsilon double-float-negative-epsilon false \
    internal-time-units-per-second lambda-list-keywords lambda-parameters-limit \
    least-negative-double-float least-negative-long-float \
    least-negative-normalized-double-float least-negative-normalized-long-float \
    least-negative-normalized-short-float least-negative-normalized-single-float \
    least-negative-short-float least-negative-single-float least-positive-double-float \
    least-positive-long-float least-positive-normalized-double-float \
    least-positive-normalized-long-float least-positive-normalized-short-float \
    least-positive-normalized-single-float least-positive-short-float \
    least-positive-single-float long-float-epsilon long-float-negative-epsilon \
    most-negative-double-float most-negative-fixnum most-negative-long-float \
    most-negative-short-float most-negative-single-float most-positive-double-float \
    most-positive-fixnum most-positive-long-float most-positive-short-float \
    most-positive-single-float multiple-values-limit nil null pi short-float-epsilon \
    short-float-negative-epsilon single-float-epsilon single-float-negative-epsilon t true";
/// bat's `identifier`, pattern 6: 48 words.
const W_VARIABLE_LANGUAGE: &str = "*applyhook* *break-on-signals* *break-on-warnings* *compile-file-pathname* \
    *compile-file-truename* *compile-print* *compile-verbose* *debug-io* *debugger-hook* \
    *default-pathname-defaults* *error-output* *evalhook* *features* *gensym-counter* \
    *load-pathname* *load-print* *load-truename* *load-verbose* *macroexpand-hook* *modules* \
    *package* *print-array* *print-base* *print-case* *print-circle* *print-escape* \
    *print-gensym* *print-length* *print-level* *print-lines* *print-miser-width* \
    *print-pprint-dispatch* *print-pretty* *print-radix* *print-readably* \
    *print-right-margin* *query-io* *random-state* *read-base* *read-default-float-format* \
    *read-eval* *read-suppress* *readtable* *standard-input* *standard-output* \
    *suppress-series-warnings* *terminal-io* *trace-output*";
/// bat's `declare-identifier`, pattern 1: 9 words.
const W_DECLARE_IDENTIFIER: &str =
    "dynamic-extent ftype ignorable ignore inline notinline optimize special type";
/// bat's `loop-body`, pattern 1: 1 words.
const W_LOOP_NAMED: &str = "named";
/// bat's `loop-body`, pattern 2: 1 words.
const W_LOOP_WITH: &str = "with";
/// bat's `loop-body`, pattern 3: 10 words.
const W_LOOP_CONTROL: &str = "always as do doing for never repeat thereis until while";
/// bat's `loop-body`, pattern 4: 6 words.
const W_LOOP_CONDITIONAL: &str = "else end if then unless when";
/// bat's `loop-body`, pattern 5: 2 words.
const W_LOOP_FLOW: &str = "finally initially";
/// bat's `loop-body`, pattern 6: 2 words.
const W_LOOP_RETURN: &str = "return return-from";
/// bat's `loop-body`, pattern 7: 1 words.
const W_LOOP_OPERATOR: &str = "=";
/// bat's `loop-body`, pattern 8: 1 words.
const W_LOOP_INTO: &str = "into";
/// bat's `loop-body`, pattern 9: 1 words.
const W_LOOP_AND: &str = "and";
/// bat's `loop-body`, pattern 10: 16 words.
const W_LOOP_OTHER: &str =
    "above across being below by downto each from in of on the to upfrom upto using";

/// Each list and what a word in it is, in the order the grammar tries them: a field already
/// set by an earlier list stays (`defun` is a function's declaration before it is one of
/// `other-declaration`'s).
const LISTS: [(&str, u32); 69] = [
    (W_CLASS, H_CLASS),
    (W_LAMBDA, H_LAMBDA),
    (W_DEFUN, H_DEFUN),
    (W_METHOD, H_METHOD),
    (W_FLET, H_FLET),
    (W_MACRO, H_MACRO),
    (W_MACROLET, H_MACROLET),
    (W_STRUCT, H_STRUCT),
    (W_TYPE, H_TYPE),
    (W_LET, H_LET),
    (W_CONSTANT, H_CONSTANT),
    (W_VAR, H_VAR),
    (W_DECLARATION, H_DECLARATION),
    (W_DECLARE, H_DECLARE),
    (W_CONDITIONAL, H_CONDITIONAL),
    (W_RETURN, H_RETURN),
    (W_DOLIST, H_DOLIST),
    (W_DO, H_DO),
    (W_CONTROL, H_CONTROL),
    (W_LOOP, H_LOOP),
    (W_COMPARISON, H_COMPARISON | O_COMPARISON),
    (W_ARITHMETIC, H_ARITHMETIC | O_ARITHMETIC),
    (W_LOGICAL, H_LOGICAL | O_LOGICAL),
    (W_BITWISE, H_BITWISE | O_BITWISE),
    (W_QUOTE, H_QUOTE),
    (W_BUILTIN, BUILTIN),
    (W_STORAGE_TYPE, I_TYPE | NOT_FUNCTION),
    (W_BOOLEAN, I_BOOLEAN),
    (W_NULL, I_NULL),
    (W_CONSTANT_LANGUAGE, I_CONSTANT | NOT_FUNCTION),
    (W_VARIABLE_LANGUAGE, I_VARIABLE),
    (W_DECLARE_IDENTIFIER, DECLARE_ID),
    (W_LOOP_NAMED, L_NAMED),
    (W_LOOP_WITH, L_WITH),
    (W_LOOP_CONTROL, L_CONTROL),
    (W_LOOP_CONDITIONAL, L_CONDITIONAL),
    (W_LOOP_FLOW, L_FLOW),
    (W_LOOP_RETURN, L_RETURN),
    (W_LOOP_OPERATOR, L_OPERATOR),
    (W_LOOP_INTO, L_INTO),
    (W_LOOP_AND, L_AND),
    (W_LOOP_OTHER, L_OTHER),
    (S_W_DEFINE, S_DEFINE),
    (S_W_DEFINE_SYNTAX, S_DEFINE_SYNTAX),
    (S_W_DEFINE_VALUES, S_DEFINE_VALUES),
    (S_W_RECORD, S_RECORD),
    (S_W_LAMBDA, S_LAMBDA),
    (S_W_CASE_LAMBDA, S_CASE_LAMBDA),
    (S_W_LET, S_LET),
    (S_W_DO, S_DO),
    (S_W_CONDITIONAL, S_CONDITIONAL),
    (S_W_LOOP, S_LOOP),
    (S_W_CONTROL, S_CONTROL),
    (S_W_LOGICAL, S_LOGICAL),
    (S_W_MODULE, S_MODULE),
    (S_W_QUOTE, S_QUOTE),
    (S_W_ARITHMETIC, S_ARITHMETIC),
    (S_W_COMPARISON, S_COMPARISON),
    (S_W_BUILTIN, S_BUILTIN),
    (S_W_CONSTANT, S_CONSTANT),
    (S_W_KEYWORD, S_KEYWORD),
    (C_W_CONDITIONAL, C_CONDITIONAL),
    (C_W_CONTROL, C_CONTROL),
    (C_W_LOOP, C_LOOP),
    (C_W_LOGICAL, C_LOGICAL),
    (C_W_MODULE, C_MODULE),
    (C_W_OPERATOR, C_OPERATOR),
    (C_W_BINDING, C_BINDING),
    (C_W_BUILTIN, C_BUILTIN),
];

// Scheme's and Racket's (R7RS small, Racket's base, a few of Guile's and Chez's).
const S_W_DEFINE: &str = "define define* define-public define-inline define-integrable define/contract \
    define/public define/private define/override define/augment define-for-syntax define-constant";
const S_W_DEFINE_SYNTAX: &str = "define-syntax define-syntax-rule define-macro define-simple-macro \
    define-syntax-parameter define-syntax-class define-syntax-parser";
const S_W_DEFINE_VALUES: &str = "define-values";
const S_W_RECORD: &str = "define-record-type struct define-struct define-structure define-class";
const S_W_LAMBDA: &str = "lambda \u{3bb} lambda* opt-lambda";
const S_W_CASE_LAMBDA: &str = "case-lambda case-lambda*";
const S_W_LET: &str = "let let* letrec letrec* let-values let*-values letrec-values fluid-let \
    parameterize let-syntax letrec-syntax with-syntax syntax-parameterize let-optionals let*-optionals \
    let-keywords match-let match-let*";
const S_W_DO: &str = "do";
const S_W_CONDITIONAL: &str = "if cond case when unless match match* guard";
const S_W_LOOP: &str = "for for* while until";
const S_W_CONTROL: &str = "begin begin0 delay delay-force lazy with-handlers with-handlers* \
    syntax-rules syntax-case syntax-parse assert";
const S_W_LOGICAL: &str = "and or";
const S_W_MODULE: &str = "require provide module module* module+ import export library define-library \
    include include-ci cond-expand only-in except-in prefix-in rename-in all-defined-out all-from-out \
    for-syntax for-label contract-out struct-out use-modules define-module";
const S_W_QUOTE: &str = "quote quasiquote syntax quasisyntax quote-syntax";
const S_W_ARITHMETIC: &str = "+ - * /";
const S_W_COMPARISON: &str = "= < > <= >=";
const S_W_BUILTIN: &str = "abs append apply assoc assq assv boolean=? boolean? bytevector \
    bytevector-append bytevector-copy bytevector-copy! bytevector-length bytevector-u8-ref \
    bytevector-u8-set! bytevector? caar cadr call-with-current-continuation call-with-port \
    call-with-values call/cc car cdar cddr cdr ceiling char->integer char-ready? char<=? char<? \
    char=? char>=? char>? char? close-input-port close-output-port close-port complex? cons \
    current-error-port current-input-port current-output-port denominator dynamic-wind eof-object \
    eof-object? eq? equal? eqv? error error-object-irritants error-object-message error-object? even? \
    exact exact-integer-sqrt exact-integer? exact? expt features file-error? floor floor-quotient \
    floor-remainder floor/ flush-output-port for-each gcd get-output-bytevector get-output-string \
    inexact inexact? input-port-open? input-port? integer->char integer? lcm length list list->string \
    list->vector list-copy list-ref list-set! list-tail list? make-bytevector make-list make-parameter \
    make-string make-vector map max member memq memv min modulo negative? newline not null? \
    number->string number? numerator odd? open-input-bytevector open-input-string \
    open-output-bytevector open-output-string output-port-open? output-port? pair? peek-char \
    peek-u8 positive? procedure? quotient raise raise-continuable rational? rationalize \
    read-bytevector read-bytevector! read-char read-error? read-line read-string read-u8 real? \
    remainder reverse round set-car! set-cdr! square string string->list string->number \
    string->symbol string->utf8 string->vector string-append string-copy string-copy! string-fill! \
    string-for-each string-length string-map string-ref string-set! string<=? string<? string=? \
    string>=? string>? string? substring symbol->string symbol=? symbol? textual-port? truncate \
    truncate-quotient truncate-remainder truncate/ u8-ready? utf8->string values vector vector->list \
    vector->string vector-append vector-copy vector-copy! vector-fill! vector-for-each vector-length \
    vector-map vector-ref vector-set! vector? with-exception-handler write-bytevector write-char \
    write-string write-u8 zero? display write write-shared write-simple read exit emergency-exit load \
    eval char-alphabetic? char-numeric? char-whitespace? char-upcase char-downcase char-foldcase \
    string-upcase string-downcase string-foldcase digit-value char-upper-case? char-lower-case? exp \
    log sin cos tan asin acos atan sqrt nan? infinite? finite? make-rectangular make-polar real-part \
    imag-part magnitude angle delete-file file-exists? command-line get-environment-variable \
    printf fprintf eprintf format displayln void add1 sub1 first second third rest last empty? filter \
    foldl foldr remove sort hash-ref hash-set! hash-set hash-update! hash-has-key? make-hash \
    string-join string-split list->set set-member? vector-sort raise-argument-error";
const S_W_CONSTANT: &str = "true false null empty";
const S_W_KEYWORD: &str = "else =>";

// Clojure's (`clojure.core`'s special forms, macros and most used functions).
const C_W_CONDITIONAL: &str = "if if-not if-let if-some when when-not when-let when-some when-first \
    cond condp case";
const C_W_CONTROL: &str = "do try catch finally throw quote var new set! monitor-enter monitor-exit \
    locking dosync delay lazy-seq lazy-cat future comment assert time io! with-out-str memfn";
const C_W_LOOP: &str = "loop recur for doseq dotimes while";
const C_W_LOGICAL: &str = "and or not";
const C_W_MODULE: &str = "ns require use import refer refer-clojure in-ns load";
const C_W_OPERATOR: &str = "+ - * / = == not= < > <= >= -> ->> some-> some->> as-> cond-> cond->> \
    doto ..";
const C_W_BINDING: &str = "let letfn binding with-open with-local-vars with-redefs";
const C_W_BUILTIN: &str = "agent alter apply assoc assoc-in atom boolean boolean? butlast cat char \
    char? class coll? commute comp compare complement completing concat conj cons constantly \
    contains? count cycle dec dedupe deliver deref disj dissoc distinct doall dorun double double? \
    drop drop-last drop-while eduction empty empty? eval even? every-pred every? ex-data ex-info \
    ex-message false? ffirst filter filterv find first flatten float fn? fnil force format \
    frequencies gensym get get-in group-by hash hash-map hash-set identical? identity ifn? inc \
    instance? int int? integer? interleave interpose into into-array iterate juxt keep \
    keep-indexed key keys keyword keyword? last list list* long macroexpand macroexpand-1 map \
    map-entry? map-indexed map? mapcat mapv max max-key memoize merge merge-with meta min min-key \
    mod name namespace nat-int? neg-int? neg? next nfirst nil? nnext not-any? not-empty \
    not-every? nth nthnext nthrest number? odd? partial partition partition-all partition-by peek \
    pop pos-int? pos? pr pr-str print printf println println-str prn prn-str promise quot rand \
    rand-int rand-nth random-sample range re-find re-matcher re-matches re-pattern re-seq \
    read-string realized? reduce reduce-kv reductions ref ref-set rem remove repeat repeatedly \
    replace reset! resolve rest reverse run! second select-keys send send-off seq seq? sequence \
    set shuffle slurp some some-fn some? sort sort-by sorted-map sorted-set spit split-at \
    split-with str string? subs subvec swap! symbol symbol? take take-last take-nth take-while \
    trampoline transduce tree-seq true? type update update-in val vals vary-meta vec vector \
    vector? volatile! vreset! vswap! with-meta zero? zipmap";

/// `def(?:[a-z]+f)?un` is a pattern, not a list: [`is_defun`]. These are the rest of its
/// alternatives.
const W_DEFUN: &str = "defgeneric define-command define-command-global define-parenscript";

/// The longest word in the lists: a longer token is none of them.
pub(super) const LONGEST: usize = 38;

/// The lists' text, one after the other, each word followed by a space.
static TEXT: [u8; TEXT_LEN] = {
    let mut t = [0u8; TEXT_LEN];
    let mut at = 0;
    let mut l = 0;
    while l < LISTS.len() {
        let s = LISTS[l].0.as_bytes();
        let mut i = 0;
        while i < s.len() {
            t[at] = s[i];
            at += 1;
            i += 1;
        }
        t[at] = b' ';
        at += 1;
        l += 1;
    }
    t
};

const TEXT_LEN: usize = {
    let mut n = 0;
    let mut l = 0;
    while l < LISTS.len() {
        n += LISTS[l].0.len() + 1;
        l += 1;
    }
    n
};

const SIZE: usize = 4096;

/// FNV-1a over the word's bytes, ASCII case folded.
const fn hash(w: &[u8], from: usize, to: usize) -> usize {
    let mut h: u32 = 0x811c_9dc5;
    let mut i = from;
    while i < to {
        h ^= w[i].to_ascii_lowercase() as u32;
        h = h.wrapping_mul(0x0100_0193);
        i += 1;
    }
    (h as usize) & (SIZE - 1)
}

/// The table as it is built, open addressed: each slot a word's place in [`TEXT`]
/// (`offset << 8 | len`, 0 for none) in its top 32 bits and what the word is in the low 32.
/// Only [`ENTRIES`] and [`SLOTS`], its compact form, are in the binary.
const fn table() -> [u64; SIZE] {
    let mut t = [0u64; SIZE];
    let mut at = 0;
    let mut l = 0;
    while l < LISTS.len() {
        let end = at + LISTS[l].0.len();
        let info = LISTS[l].1;
        while at < end {
            let mut e = at;
            while e < end && TEXT[e] != b' ' {
                e += 1;
            }
            if e > at {
                let mut h = hash(&TEXT, at, e);
                loop {
                    let slot = t[h];
                    if slot == 0 {
                        t[h] = ((at as u64) << 40) | (((e - at) as u64) << 32) | info as u64;
                        break;
                    }
                    let (o, n) = ((slot >> 40) as usize, ((slot >> 32) & 0xff) as usize);
                    if n == e - at && same(&TEXT, o, at, n) {
                        t[h] = slot | merge(slot as u32, info) as u64;
                        break;
                    }
                    h = (h + 1) & (SIZE - 1);
                }
            }
            at = e + 1;
        }
        at = end + 1;
        l += 1;
    }
    t
}

/// How many words [`table`] holds.
const WORDS: usize = {
    let table = table();
    let mut n = 0;
    let mut h = 0;
    while h < SIZE {
        n += (table[h] != 0) as usize;
        h += 1;
    }
    n
};

/// [`table`]'s words, in its order: 8 bytes a word, where the table's empty slots took 8 too.
static ENTRIES: [u64; WORDS] = {
    let table = table();
    let mut e = [0u64; WORDS];
    let (mut h, mut i) = (0, 0);
    while h < SIZE {
        if table[h] != 0 {
            e[i] = table[h];
            i += 1;
        }
        h += 1;
    }
    e
};

/// The open-addressed slots: a word's index in [`ENTRIES`] plus one, 0 for none; sparse
/// enough (below two in five) that a miss, a head of no list, stops at once.
static SLOTS: [u16; SIZE] = {
    let table = table();
    let mut t = [0u16; SIZE];
    let (mut h, mut i) = (0, 0);
    while h < SIZE {
        if table[h] != 0 {
            i += 1;
            t[h] = i as u16;
        }
        h += 1;
    }
    t
};

const fn same(t: &[u8], a: usize, b: usize, n: usize) -> bool {
    let mut i = 0;
    while i < n {
        if t[a + i] != t[b + i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The fields of a word's info that hold one value, the first list's that sets them.
const FIELDS: u32 = HEAD | IDENT | LOOP_WORD | OPERATOR | SCHEME_HEAD | SCHEME_IDENT | CLOJURE_HEAD;

/// `info`'s fields that `old` doesn't set yet.
const fn merge(old: u32, info: u32) -> u32 {
    let mut add = info & !FIELDS;
    let fields = [
        HEAD,
        IDENT,
        LOOP_WORD,
        OPERATOR,
        SCHEME_HEAD,
        SCHEME_IDENT,
        CLOJURE_HEAD,
    ];
    let mut i = 0;
    while i < fields.len() {
        if old & fields[i] == 0 {
            add |= info & fields[i];
        }
        i += 1;
    }
    add
}

/// What word `w` is, ASCII case ignored: 0 for none of the lists'.
#[inline]
pub(super) fn lookup(w: &[u8]) -> u32 {
    if w.len() > LONGEST {
        return 0;
    }
    let mut h = hash(w, 0, w.len());
    loop {
        let i = SLOTS[h] as usize;
        if i == 0 {
            return 0;
        }
        let slot = ENTRIES[i - 1];
        let (o, n) = ((slot >> 40) as usize, ((slot >> 32) & 0xff) as usize);
        if n == w.len() && TEXT[o..o + n].eq_ignore_ascii_case(w) {
            return slot as u32;
        }
        h = (h + 1) & (SIZE - 1);
    }
}

/// What word `w` is in Clojure, which is case-sensitive: the lists' words are lowercase, so a
/// token with an uppercase letter is none of them.
#[inline]
pub(super) fn clojure_lookup(w: &[u8]) -> u32 {
    if w.iter().any(u8::is_ascii_uppercase) {
        return 0;
    }
    lookup(w)
}

/// `def(?:[a-z]+f)?un`, ASCII case ignored.
pub(super) fn is_defun(w: &[u8]) -> bool {
    let n = w.len();
    if n < 5 || !w[..3].eq_ignore_ascii_case(b"def") || !w[n - 2..].eq_ignore_ascii_case(b"un") {
        return false;
    }
    let mid = &w[3..n - 2];
    mid.is_empty()
        || (mid.len() >= 2
            && mid[mid.len() - 1].eq_ignore_ascii_case(&b'f')
            && mid[..mid.len() - 1].iter().all(u8::is_ascii_alphabetic))
}

/// The word kinds the scanner reads off a lookup.
pub(super) fn head(info: u32) -> u32 {
    info & HEAD
}

pub(super) fn ident(info: u32) -> u32 {
    info & IDENT
}

pub(super) fn loop_word(info: u32) -> u32 {
    info & LOOP_WORD
}

pub(super) fn operator(info: u32) -> u32 {
    info & OPERATOR
}

pub(super) fn scheme_head(info: u32) -> u32 {
    info & SCHEME_HEAD
}

pub(super) fn scheme_ident(info: u32) -> u32 {
    info & SCHEME_IDENT
}

pub(super) fn clojure_head(info: u32) -> u32 {
    info & CLOJURE_HEAD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_is_found_with_its_lists() {
        let mut words = 0;
        for (list, info) in LISTS {
            for w in list.split(' ') {
                let got = lookup(w.as_bytes());
                assert_ne!(got, 0, "{w}");
                // a field is the first list's that sets it
                for field in [
                    HEAD,
                    IDENT,
                    LOOP_WORD,
                    OPERATOR,
                    SCHEME_HEAD,
                    SCHEME_IDENT,
                    CLOJURE_HEAD,
                ] {
                    if info & field != 0 {
                        assert_ne!(got & field, 0, "{w}");
                    }
                }
                let rest = info & !FIELDS;
                assert_eq!(got & rest, rest, "{w}");
                assert_eq!(lookup(w.to_ascii_uppercase().as_bytes()), got, "{w}");
                assert!(w.len() <= LONGEST);
                words += 1;
            }
        }
        assert!(words > 1000);
        assert_eq!(head(lookup(b"defun")), H_DECLARATION);
        assert_eq!(head(lookup(b"let*")), H_LET);
        assert_eq!(head(lookup(b"mapcar")), H_CONTROL);
        assert_ne!(lookup(b"car") & BUILTIN, 0);
        assert_eq!(ident(lookup(b"nil")), I_NULL);
        assert_ne!(lookup(b"nil") & NOT_FUNCTION, 0);
        assert_eq!(loop_word(lookup(b"and")), L_AND);
        assert_eq!(operator(lookup(b"and")), O_LOGICAL);
        assert_eq!(lookup(b"no-such-word"), 0);
        assert_eq!(lookup(b"de"), 0);
        assert_eq!(clojure_head(clojure_lookup(b"if-some")), C_CONDITIONAL);
        assert_eq!(clojure_head(clojure_lookup(b"->>")), C_OPERATOR);
        assert_eq!(clojure_head(clojure_lookup(b"let")), C_BINDING);
        assert_ne!(clojure_lookup(b"re-matches") & C_BUILTIN, 0);
        assert_eq!(clojure_lookup(b"If-Some"), 0, "Clojure is case-sensitive");
        // Common Lisp's `when` and Clojure's share a slot, each dialect its own field
        assert_eq!(head(lookup(b"when")), H_CONDITIONAL);
        assert_eq!(clojure_head(lookup(b"when")), C_CONDITIONAL);
    }

    /// The slots stay sparse enough that a miss, the common case, probes a slot or two; each
    /// word is in one, and `u16` indices reach every entry.
    #[test]
    fn the_slots_stay_sparse() {
        let used = SLOTS.iter().filter(|&&s| s != 0).count();
        assert_eq!(used, WORDS);
        assert!(used * 5 <= SIZE * 2, "{used} of {SIZE}");
        assert!(WORDS < usize::from(u16::MAX));
    }

    #[test]
    fn defun_pattern() {
        for w in ["defun", "DEFUN", "defsubstfun", "defabfun"] {
            assert!(is_defun(w.as_bytes()), "{w}");
        }
        for w in ["defu", "defxun", "deffun", "def-fun", "defunx"] {
            assert!(!is_defun(w.as_bytes()), "{w}");
        }
    }
}
