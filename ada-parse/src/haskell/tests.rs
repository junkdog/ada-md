//! Haskell's readings on [`super`]'s scanner (X-136): its line and block APIs directly, and
//! through [`crate::lex`] by [`Lang::Haskell`]; Literate Haskell's host.

use super::*;
use crate::Lang;
use crate::tests::marked;

/// The tokens of `lines` on `f` (Haskell's [`super::line`] or Literate Haskell's), each with
/// its scopes.
fn of_with(f: fn(&[u8], &mut State, &mut Out), lines: &[&str]) -> Vec<(String, String)> {
    let mut state = State::default();
    let mut v = Vec::new();
    for l in lines {
        let mut spans = Vec::new();
        f(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
        for s in &spans {
            let text = &l[s.start as usize..s.end as usize];
            v.push((text.to_owned(), scopes(s.kind).join(" ")));
        }
    }
    v
}

fn of(lines: &[&str]) -> Vec<(String, String)> {
    of_with(line, lines)
}

fn has(lines: &[&str], text: &str, scope: &str) {
    let toks = of(lines);
    assert!(
        toks.iter().any(|(t, s)| t == text && s == scope),
        "{text:?} as {scope:?} in {toks:#?}"
    );
}

/// `text` is never `scope` in `lines`.
fn never(lines: &[&str], text: &str, scope: &str) {
    let toks = of(lines);
    assert!(
        toks.iter().all(|(t, s)| t != text || s != scope),
        "{text:?} never {scope:?} in {toks:#?}"
    );
}

/// `text` is plain wherever it stands in `lines`.
fn plain(lines: &[&str], text: &str) {
    let toks = of(lines);
    assert!(
        toks.iter().all(|(t, _)| t != text),
        "{text:?} plain in {toks:#?}"
    );
}

const OP: &str = "keyword.operator.haskell";
const KW: &str = "keyword.other.haskell";
const COMMENT: &str = "comment.line.double-dash.haskell";
const STR: &str = "string.quoted.double.haskell";
const TYPE: &str = "storage.type.haskell";
const VAR: &str = "variable.other.generic-type.haskell";
const FUNC: &str = "entity.name.function.haskell";
const CONST: &str = "constant.other.haskell";
const QUASI: &str = "string.quasiquoted.haskell";

#[test]
fn lex_takes_this_scanner() {
    let m = marked(Lang::Haskell, &["main = putStrLn \"hi\""]);
    assert!(m.iter().any(|(t, _)| *t == "\"hi\"" || *t == "hi"), "{m:?}");
}

#[test]
fn signatures_run_to_their_layout() {
    let l = [
        "f :: (Monad m, Show a)",
        "  => a",
        "  -- a comment",
        "",
        "  -> m ()",
        "f x = pure ()",
        "  where",
        "    go :: forall b. b -> b",
        "    go = id",
        "(<+>) :: Int -> Int",
        "query :: Connection -> Query",
    ];
    has(&l, "f", FUNC);
    has(&l, "::", "keyword.other.double-colon.haskell");
    has(&l, "Monad", TYPE);
    has(&l, "m", VAR);
    has(&l, "=>", "keyword.other.big-arrow.haskell");
    has(&l, "->", "keyword.other.arrow.haskell");
    has(&l, "()", "support.constant.unit.haskell");
    has(&l, "forall", KW);
    has(&l, "go", FUNC);
    has(&l, "(<+>)", FUNC);
    // a name a SQL call has elsewhere
    has(&l, "query", FUNC);
    has(&l, "Connection", TYPE);
    // the equation at the signature's column ends it
    has(&l, "=", OP);
    never(&l, "x", VAR);
}

const DC: &str = "keyword.other.double-colon.haskell";
const INHERITED: &str = "entity.other.inherited-class.haskell";

#[test]
fn record_fields_are_signatures() {
    // on one line: each field's type to its `,` or `}`, outside its brackets
    let l = ["data P = P { px :: Int, py, pz :: Maybe (Int, Int) } deriving Show"];
    has(&l, "px", FUNC);
    has(&l, "py", FUNC);
    has(&l, "pz", FUNC);
    has(&l, "Int", TYPE);
    has(&l, "Maybe", TYPE);
    has(&l, "P", CONST);
    never(&l, "P", TYPE);
    has(&l, "deriving", KW);
    // leading commas, a type on the line after its `::`, a pragma and a comment in a type
    let l = [
        "data R = R",
        "  { rA :: !Int",
        "  , rB :: {-# UNPACK #-} !Double -- ^ doc",
        "  , rC ::",
        "      Map Int",
        "        String",
        "  , rD :: forall a. a -> [a]",
        "  }",
        "  deriving (Eq)",
        "x = R { rA = 1 }",
    ];
    for f in ["rA", "rB", "rC", "rD"] {
        has(&l, f, FUNC);
    }
    for t in ["Double", "Map", "String"] {
        has(&l, t, TYPE);
    }
    has(&l, "a", VAR);
    has(
        &l,
        "UNPACK",
        "meta.preprocessor.haskell keyword.other.preprocessor.haskell",
    );
    has(&l, " ^ doc", COMMENT);
    has(&l, "Eq", INHERITED);
    // a record's construction isn't a field's declaration
    never(&l, "x", FUNC);
    never(&l, "R", TYPE);
    // trailing commas, the record's `}` on a field's line
    let l = [
        "data S = S {",
        "    sA :: Int,",
        "    sB :: Bool }",
        "y = 1",
    ];
    has(&l, "sA", FUNC);
    has(&l, "Bool", TYPE);
    has(&l, ",", "punctuation.separator.comma.haskell");
    never(&l, "y", VAR);
    // explicit braces' `;` ends a signature's type
    let l = ["f = let { g :: Int -> Int; g = id } in g"];
    has(&l, "g", FUNC);
    has(&l, "Int", TYPE);
    never(&l, "id", VAR);
    never(&l, "in", VAR);
}

#[test]
fn constructors_and_name_lists_are_signatures() {
    let l = [
        "data T a where",
        "  TInt :: Int -> T Int",
        "  TPair, TPair' :: T a -> T (a, b)",
        "  TRec :: { tf :: Int, tg :: (Int, Int) } -> T Int",
        "class C a where",
        "  (==.), (/=.) :: a -> a -> Bool",
        "  foo, bar :: a",
        "f, g :: Int",
        "f = id",
    ];
    has(&l, "TInt", CONST);
    has(&l, "tg", FUNC);
    has(&l, "TPair'", CONST);
    has(&l, "T", TYPE);
    has(&l, "->", "keyword.other.arrow.haskell");
    // a GADT record's fields in a constructor's type, the type going on
    has(&l, "tf", FUNC);
    has(&l, "(==.)", FUNC);
    has(&l, "(/=.)", FUNC);
    has(&l, "foo", FUNC);
    has(&l, "bar", FUNC);
    has(&l, "g", FUNC);
    has(&l, "Bool", TYPE);
    never(&l, "id", VAR);
    // a constructor at a line's start without `::` is a constant
    let l = ["  Just x :: Maybe Int", "Foo.bar :: Int"];
    has(&l, "Just", CONST);
    never(&l, "Foo", TYPE);
}

#[test]
fn signatures_after_a_keyword() {
    let l = [
        "class C a where",
        "  default foo :: (Show a)",
        "    => a",
        "f = go where go :: Int",
        "g = let k, j :: Int",
        "        k = 1 in k",
        "pattern Pair :: a -> b -> (a, b)",
        "pattern = 1",
    ];
    for n in ["foo", "go", "k", "j"] {
        has(&l, n, FUNC);
    }
    has(&l, "Show", TYPE);
    has(&l, "Pair", CONST);
    has(&l, "pattern", KW);
    // `pattern` a name elsewhere
    let toks = of(&l);
    assert_eq!(
        toks.iter().filter(|(t, _)| t == "pattern").count(),
        1,
        "{toks:#?}"
    );
    has(&l, "in", KW);
}

#[test]
fn a_double_colon_starting_a_line_opens_a_signature() {
    let l = [
        "longName",
        "  :: (Show a)",
        "  => a",
        "  -> Int",
        "longName x = 1",
    ];
    has(&l, "::", DC);
    has(&l, "Show", TYPE);
    has(&l, "a", VAR);
    has(&l, "->", "keyword.other.arrow.haskell");
    never(&l, "x", VAR);
    has(&l, "=", OP);
}

#[test]
fn annotations_read_their_types() {
    let l = [
        "h x = (fromIntegral x :: Double) + read s :: Double",
        "main = do",
        "  n :: Int <- readLn",
        "  let m = read s :: Int",
        "      k = m",
        "  pure (a :: Int, b)",
        "  case v :: Maybe Int of",
        "    _ -> if q :: Bool then r else w",
        "type family F a :: Type where",
        "foreign import ccall \"x\" c_x :: CInt -> IO ()",
    ];
    has(&l, "::", DC);
    never(&l, "::", OP);
    for t in ["Double", "Int", "Maybe", "Bool", "Type", "CInt", "IO"] {
        has(&l, t, TYPE);
    }
    has(&l, "()", "support.constant.unit.haskell");
    // each type ends where the expression goes on
    has(&l, "+", OP);
    has(&l, "<-", OP);
    for k in ["of", "then", "else", "where"] {
        has(
            &l,
            k,
            KW_OR_CONTROL[usize::from(matches!(k, "then" | "else"))],
        );
    }
    for v in ["readLn", "k", "m", "b", "r", "w", "s"] {
        never(&l, v, VAR);
    }
}

const KW_OR_CONTROL: [&str; 2] = [KW, "keyword.control.haskell"];

#[test]
fn type_applications() {
    let l = [
        "x = show @Int 1 <> f @(Maybe a) y @M.Map @k' @[Bool] @() z",
        "g p@(Just q) = h q",
    ];
    for t in ["Int", "Maybe", "M", "Map", "Bool"] {
        has(&l, t, TYPE);
    }
    has(&l, "a", VAR);
    has(&l, "k", VAR);
    has(&l, "()", "support.constant.unit.haskell");
    // the type ends with its bracket, an as-pattern's `@` no type application's
    for v in ["y", "z", "q"] {
        never(&l, v, VAR);
    }
    never(&l, "Just", TYPE);
}

#[test]
fn deriving_clauses() {
    let l = [
        "data S = S Int",
        "  deriving stock (Eq, Generic)",
        "  deriving anyclass (ToJSON)",
        "  deriving newtype Num",
        "  deriving Data.Data.Data",
        "  deriving (Semigroup, Monoid) via (Sum Int)",
        "  deriving",
        "    ( Ord",
        "    , Read",
        "    )",
        "    via Down S",
        "deriving instance Show (T a)",
        "deriving stock instance Eq P",
        "deriving via (Max Int) instance Semigroup M",
        "deriving via",
        "  (Min Int)",
        "  instance",
        "    (Ord a) =>",
        "    Monoid (W a)",
        "data instance MVector s (a, b)",
        "    = MV_2 !Int",
        "type instance F Int = Bool",
        "x = stock",
    ];
    for k in ["stock", "anyclass", "newtype", "via", "instance"] {
        has(&l, k, KW);
    }
    for c in [
        "Eq",
        "Generic",
        "ToJSON",
        "Num",
        "Data",
        "Semigroup",
        "Monoid",
        "Ord",
        "Read",
    ] {
        has(&l, c, INHERITED);
    }
    never(&l, "Num", CONST);
    // `via`'s type, a standalone deriving's instance head
    for t in ["Sum", "Down", "Show", "Max", "M", "Min", "Monoid", "W"] {
        has(&l, t, TYPE);
    }
    // a data instance's constructors aren't its head's
    has(&l, "MV_2", CONST);
    // a type instance's right side a type
    has(&l, "Bool", TYPE);
    has(&l, "MVector", TYPE);
    // the clause ends where its classes do
    has(&l, "=", OP);
    never(&l, "x", VAR);
    let toks = of(&l);
    assert_eq!(
        toks.iter().filter(|(t, s)| t == "stock" && s == KW).count(),
        2,
        "{toks:#?}"
    );
}

#[test]
fn heads() {
    let l = [
        "module Data.Foo",
        "  ( Foo(..), bar, (<+>)",
        "  , Baz(A,",
        "        B)",
        "  , module X",
        "#if MIN_VERSION_base(4,8,0)",
        "  , (<&>)",
        "#endif",
        "  ) where",
        "import qualified Data.Map as M",
        "import Data.List (sortBy, foldl')",
        "class (Eq a) => Container f a where",
        "  empty :: f a",
        "class Plain a",
        "x = a",
        "instance Show Wrap where",
        "data T = T deriving (Show, Eq)",
    ];
    has(&l, "Data.Foo", "support.other.module.haskell");
    has(&l, "Foo", TYPE);
    has(&l, "(..)", "entity.name.function.infix.haskell");
    has(&l, "bar", FUNC);
    has(&l, "(<+>)", "entity.name.function.infix.haskell");
    // an inner list's names, across a line, are plain as bat's `\(.*?\)` has them
    never(&l, "B", TYPE);
    has(&l, "module", FUNC);
    // a CPP line in a list leaves the list as it was
    has(&l, "(<&>)", "entity.name.function.infix.haskell");
    never(&l, "MIN_VERSION_base", TYPE);
    has(&l, "where", KW);
    has(&l, "qualified", KW);
    has(&l, "as", KW);
    has(&l, "M", "support.other.module.haskell");
    has(&l, "sortBy", FUNC);
    has(&l, "Eq", "support.class.prelude.haskell");
    has(&l, "Container", "entity.other.inherited-class.haskell");
    has(&l, "f", VAR);
    // a class's head without `where` ends with its declaration
    never(&l, "x", VAR);
    has(&l, "Show", TYPE);
    has(&l, "Wrap", TYPE);
    has(&l, "Show", "entity.other.inherited-class.haskell");
    has(&l, "T", CONST);
}

#[test]
fn operators_and_comments_as_haskell_lexes_them() {
    let l = [
        "a --> b <*> c && d @ e ^. f",
        "x = y -- a comment",
        "--- three dashes",
        "z = w --| an operator",
        "{- a {- nested -} comment -} v",
        "{-# INLINE foo #-}",
        "g = (&&) . (<&>) . (,)",
        "(.&&.) :: P -> P",
    ];
    has(&l, "-->", OP);
    has(&l, "<*>", OP);
    has(&l, "&&", OP);
    has(&l, "@", OP);
    has(&l, "^.", OP);
    has(&l, " a comment", COMMENT);
    has(&l, "- three dashes", COMMENT);
    has(&l, "--|", OP);
    has(
        &l,
        "{-",
        "comment.block.haskell punctuation.definition.comment.begin.haskell",
    );
    has(&l, " nested ", "comment.block.haskell");
    has(&l, " comment ", "comment.block.haskell");
    has(
        &l,
        "INLINE",
        "meta.preprocessor.haskell keyword.other.preprocessor.haskell",
    );
    has(&l, " foo #-}", "meta.preprocessor.haskell");
    has(&l, "(&&)", "entity.name.function.infix.haskell");
    has(&l, "(<&>)", "entity.name.function.infix.haskell");
    has(&l, "(,)", "entity.name.function.infix.haskell");
    has(&l, "(.&&.)", FUNC);
    has(&l, "P", TYPE);
}

#[test]
fn literals() {
    let l = [
        "n = 0x1F + 0o17 + 0b101 + 1_000 + 1.5e-3 + 2e10 + 0x1.8p3 + 0xAp-2",
        "c = 'x' : '\\n' : '\\x41' : '\\65' : '\\^C' : 'λ' : []",
        "s = \"a \\\"b\\\" \\1234 \\SOH \\^C ^C\"",
        "t = \"gap \\",
        "    \\continues\" ++ x",
        "p = 'Just ''Maybe x' Foo'",
    ];
    has(
        &l,
        "0b",
        "constant.numeric.integer.binary.haskell punctuation.definition.numeric.base.haskell",
    );
    has(&l, "1_000", "constant.numeric.integer.decimal.haskell");
    has(&l, "1F", "constant.numeric.integer.hexadecimal.haskell");
    has(&l, "1.8p3", "constant.numeric.float.hexadecimal.haskell");
    has(&l, "Ap-2", "constant.numeric.float.hexadecimal.haskell");
    has(&l, "x", "string.quoted.single.haskell");
    has(
        &l,
        "\\65",
        "string.quoted.single.haskell constant.character.escape.octal.haskell",
    );
    has(
        &l,
        "\\^C",
        "string.quoted.single.haskell constant.character.escape.control.haskell",
    );
    has(&l, "λ", "string.quoted.single.haskell");
    has(
        &l,
        "\\SOH",
        "string.quoted.double.haskell constant.character.escape.haskell",
    );
    has(
        &l,
        "\\^C",
        "string.quoted.double.haskell constant.character.escape.control.haskell",
    );
    has(&l, " ^C", STR);
    has(&l, "continues", STR);
    has(&l, "++", OP);
    has(&l, "Foo'", CONST);
    plain(&l, "x'");
}

#[test]
fn quasi_quotes() {
    let l = [
        "a = [r|one|] <> [r|two|]",
        "b = [PG.sql|SELECT 1|]",
        "c = [aesonQQ|",
        "  {\"a\": 1}",
        "|]",
        "d = [x|x <- xs]",
        "e = [e| 1 + $(f) |]",
        "f = [d|",
        "  g :: Int",
        "  g = 1",
        "  |] ++ [|| h ||]",
    ];
    has(&l, "one", QUASI);
    has(&l, "two", QUASI);
    has(&l, "[PG.sql|", "keyword.other.quasibracket.haskell");
    has(
        &l,
        "SELECT",
        "string.quasiquoted.haskell keyword.other.DML.sql",
    );
    has(&l, "  {\"a\": 1}", QUASI);
    never(&l, "[x|", "keyword.other.quasibracket.haskell");
    // a quote of Template Haskell's is code
    has(&l, "[e|", "keyword.other.quasibracket.haskell");
    has(&l, "+", OP);
    has(&l, "$(", "keyword.other.splice.haskell");
    has(&l, "g", FUNC);
    has(&l, "1", "constant.numeric.integer.decimal.haskell");
    has(&l, "|]", "keyword.other.quasibracket.haskell");
    has(&l, "[||", "keyword.other.quasibracket.haskell");
    has(&l, "||]", "keyword.other.quasibracket.haskell");
    has(&l, "++", OP);
}

#[test]
fn sql_and_regexes() {
    let l = [
        "rows <- query conn \"SELECT id FROM users WHERE age > ?\" (Only n)",
        "_ <- execute_ conn",
        "  \"DELETE FROM sessions\"",
        "rawSql \"SELECT ?? FROM person\" []",
        "xs <- query_ c [sql|",
        "  SELECT a -- c",
        "  FROM t |]",
        "ok = s =~ \"[a-z]+\"",
        "r = mkRegex \"^a(b|c)$\"",
        "q = [re|^(\\d+)$|]",
        "h = query \"name\" x",
        "m = DB.query db \"SELECT \\",
        "    \\FROM t\"",
    ];
    let sql = "string.quoted.double.haskell keyword.other.DML.sql";
    has(&l, "SELECT", sql);
    has(&l, "DELETE", sql);
    has(&l, "FROM", sql);
    has(
        &l,
        "FROM",
        "string.quasiquoted.haskell keyword.other.DML.sql",
    );
    has(
        &l,
        "[",
        "string.quoted.double.haskell constant.other.character-class.set.regexp \
         punctuation.definition.character-class.begin.regexp",
    );
    has(
        &l,
        "(",
        "string.quoted.double.haskell punctuation.definition.group.begin.regexp",
    );
    has(
        &l,
        "|",
        "string.quoted.double.haskell keyword.operator.or.regexp",
    );
    has(
        &l,
        "\\d",
        "string.quasiquoted.haskell constant.other.character-class.escape.backslash.regexp",
    );
    has(
        &l,
        "^",
        "string.quasiquoted.haskell keyword.control.anchor.regexp",
    );
    // `query` with no connection before a string that isn't one: SQL all the same
    has(
        &l,
        "FROM",
        "string.quoted.double.haskell keyword.other.DML.sql",
    );
}

#[test]
fn cpp_lines() {
    let l = ["#if MIN_VERSION_base(4,8,0)", "x = 1", "  #  endif"];
    has(
        &l,
        "#",
        "meta.preprocessor.c punctuation.definition.preprocessor.c",
    );
    has(&l, "if", "meta.preprocessor.c");
    has(&l, "MIN_VERSION_base", CONST);
    has(&l, "  ", "meta.preprocessor.c");
}

#[test]
fn literate_haskell() {
    let l = [
        "Some prose with a > in it.",
        "",
        "> main :: IO ()",
        "> main = putStrLn \"hi\" -- c",
        "",
        "\\begin{code}",
        "f :: Int",
        "f = 1",
        "\\end{code}",
        "More prose, \"not a string\".",
        "``` haskell",
        "g = 'x'",
        "```",
        "h = \"prose\"",
    ];
    let toks = of_with(lhs::line, &l);
    let has = |text: &str, scope: &str| {
        assert!(
            toks.iter().any(|(t, s)| t == text && s == scope),
            "{text:?} as {scope:?} in {toks:#?}"
        );
    };
    has(">", "punctuation.definition.bird-track.haskell");
    has("main", FUNC);
    has("hi", STR);
    has(" c", COMMENT);
    has("begin", "support.function.be.latex");
    has("code", "variable.parameter.function.latex");
    has("f", FUNC);
    has("x", "string.quoted.single.haskell");
    for prose in ["not a string", "prose", "Some", "More"] {
        assert!(
            toks.iter().all(|(t, _)| t != prose),
            "{prose:?} plain in {toks:#?}"
        );
    }
}

/// Each line's spans and the state after it, a line at a time.
fn lines(text: &str) -> Vec<(Vec<crate::Span>, State)> {
    let mut state = State::default();
    let mut out = Vec::new();
    for l in text.lines() {
        let mut spans = Vec::new();
        line(l.as_bytes(), &mut state, &mut Out::new(&mut spans));
        out.push((spans, state));
    }
    out
}

#[test]
fn a_block_lexes_as_its_lines() {
    // every field a line hands the next: a comment's depth, a pragma, a quasi-quote's kind and
    // SQL's context, a regex's state, a string's gap (of each kind), a head with its indent and
    // its list's depth, a SQL call's place
    let deep = "{- ".repeat(300);
    let text = format!(
        "{deep}\n{}\n{{-# LANGUAGE\n  X #-}}\nf :: Int\n  -> Int\nmodule A (\n (B(\n C)), d\n ) where\n\
         class C a\n  => D a\nx = [sql| SELECT /* a\n */ 'b\n c' |]\ny = [re|(a\n[b|]\n\
         s = \"a \\\n  \\b\"\nq = query c \"SELECT \\\n \\1\" ++ execute\n  conn\n  \"DELETE\"\n\
         r = x =~ \"(a\\\n \\b)\"\nimport A (\n b)\nt = [d| [e|\n x\n |]\n",
        "-} ".repeat(300)
    );
    let one = lines(&text);
    let mut state = State::default();
    let mut all = Vec::new();
    let mut v = Vec::new();
    block(&mut state, &mut v, &mut |c| {
        for l in text.lines() {
            all.push(c.next(l.as_bytes()).to_vec());
        }
        None
    });
    assert_eq!(all, one.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>());
    assert_eq!(state, one.last().unwrap().1);
    assert_eq!(
        one[0].1.word() >> 16 & 255,
        255,
        "the comment's depth saturates"
    );
}

#[test]
fn spans_stay_in_order_on_every_line() {
    let text = "a = [sql|'x|] ++ \"\\\n\\\" [re|[\n|]\nx = 'λ' ∷ → ⇒ \"∙\" =~ \"∙\"\n>\\k<) (Ꭰ>\\k< `Ꭰx` Ꭰ'\n\
                {-# A \"b\" #-} `M.f` (,) (∷)\n#\n# x\nq = query c \"a\\SOH\\&\\^\" \"b\"\n\
                r = s =~ \"a\\\\\\^\\\nq = query c \"SELECT \\^\\\n\
                  C :: { a :: Int, b, c :: T } -> T\n  , d :: X, e :: Y }\nx = (a :: Int, b)\n";
    for (l, (spans, _)) in text.lines().zip(lines(text)) {
        let mut at = 0;
        for s in &spans {
            assert!(
                s.start as usize >= at && s.end >= s.start && s.end as usize <= l.len(),
                "{l:?}: {spans:?}"
            );
            assert!(l.is_char_boundary(s.start as usize) && l.is_char_boundary(s.end as usize));
            at = s.end as usize;
        }
    }
}

#[test]
fn long_lines_lex_in_linear_time() {
    for unit in [
        "[a|<-",
        "[a|",
        "[sql|",
        "[re|(",
        "{- ",
        "{-# ",
        "\"\\",
        "\" \\  \\",
        "'\\",
        "`a.",
        "(",
        "((+)",
        "query c ",
        "query       ",
        "=~ \"",
        "mkRegex \"",
        "f :: ",
        "{ a :: ",
        "{a,",
        ", a :: T, a :: ",
        "deriving ",
        "f :: (",
        "x :: (",
        "x :: [",
        " @(",
        " @A.",
        "--> ",
        "#",
        "a `b` ",
    ] {
        let l = unit.repeat(20_000);
        let t = std::time::Instant::now();
        lines(&l);
        assert!(t.elapsed().as_millis() < 200, "{unit:?}: {:?}", t.elapsed());
    }
}
