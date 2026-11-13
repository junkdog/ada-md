//! Code by its lines (FEAT-020): each line of the sample is matched against a table of signals, a
//! regex and the languages it points to with a weight (3 a sign one language has alone, 2 a
//! strong one, 1 a shared or weak one). A signal counts at most [`CAP`] times, so a common one
//! can't outweigh a rare sure one. The language with the highest total wins, the earlier in
//! [`ORDER`] on a tie, if it reaches [`MIN_SCORE`]. Lines inside a markdown fence count only
//! for markdown: a README's examples aren't its language.
//!
//! The C family is one candidate here (`C`); [`ada_parse::Lang::for_header`] then tells C, C++,
//! Objective-C and Objective-C++ apart, as it does for a `.h`.

use std::sync::LazyLock;

use regex::bytes::{RegexSet, RegexSetBuilder};

/// How many times a signal counts.
const CAP: u32 = 4;

/// The least total a language needs.
const MIN_SCORE: u32 = 5;

/// Only a line's first bytes are matched: a minified line says little more past them.
const LINE: usize = 240;

/// Markdown's label among the candidates: not a language of a syntax.
pub(super) const MARKDOWN: &str = "Markdown";

const RUST: &str = "Rust";
const GO: &str = "Go";
const SWIFT: &str = "Swift";
const KOTLIN: &str = "Kotlin";
const SCALA: &str = "Scala";
const GROOVY: &str = "Groovy";
const JAVA: &str = "Java";
const CSHARP: &str = "C#";
/// The C family, told apart by `Lang::for_header`.
pub(super) const CFAM: &str = "C";
const PYTHON: &str = "Python";
const RUBY: &str = "Ruby";
const ELIXIR: &str = "Elixir";
const LUA: &str = "Lua";
const PERL: &str = "Perl";
const BASH: &str = "Bourne Again Shell (bash)";
const PS: &str = "PowerShell";
const SQL: &str = "SQL";
const HASKELL: &str = "Haskell";
const NIX: &str = "Nix";
const TF: &str = "Terraform";
const NGINX: &str = "nginx";
const CMAKE: &str = "CMake";
const MAKE: &str = "Makefile";
const DOCKER: &str = "Dockerfile";
const CSS: &str = "CSS";
const SCSS: &str = "SCSS";
const JS: &str = "JavaScript (Babel)";
const TS: &str = "TypeScript";
const TSX: &str = "TypeScriptReact";
const CLOJURE: &str = "Clojure";
const ELISP: &str = "Emacs Lisp";
const LISP: &str = "Lisp";
const SCHEME: &str = "Scheme";
const RACKET: &str = "Racket";
const HTML: &str = "HTML";
const XML: &str = "XML";
const MD: &str = MARKDOWN;

/// Every candidate, in the order that breaks a tie: the more common or the more general first
/// (JavaScript before TypeScript, TypeScript before TSX, Java before Groovy, Groovy before
/// Kotlin and Scala: a build script that shows neither is more often Gradle's Groovy).
pub(super) const ORDER: &[&str] = &[
    MD, PYTHON, JS, TS, TSX, JAVA, CFAM, CSHARP, GO, RUST, BASH, RUBY, GROOVY, KOTLIN, SWIFT,
    SCALA, LUA, PERL, PS, SQL, HASKELL, ELIXIR, NIX, TF, NGINX, CMAKE, MAKE, DOCKER, CSS, SCSS,
    CLOJURE, ELISP, LISP, SCHEME, RACKET, HTML, XML,
];

type Signal = (&'static str, &'static [(&'static str, u8)]);

/// The signals, by language. A regex is matched against a line as it is (`^\s*` where
/// indentation is allowed).
const SIGNALS: &[Signal] = &[
    // markdown: a fence, a table's rule and a link are sure; the rest weak
    (r"^ {0,3}(```|~~~)", &[(MD, 4)]),
    (
        r"^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?\s*$",
        &[(MD, 4)],
    ),
    (
        r"\[[^\]]+\]\((https?://|\.{0,2}/|#)[^)\s]*\)|\[\[[^\]]+\]\]",
        &[(MD, 2)],
    ),
    (r"^#{2,6} \S", &[(MD, 1)]),
    (r"^( {2,3})?[-*+] (\[[ x]\] )?[A-Z`*\[]", &[(MD, 1)]),
    (r"^\s{0,3}\d{1,3}\. [A-Z`*\[]", &[(MD, 1)]),
    (r"^ {0,3}> \S", &[(MD, 1)]),
    // inline markup, not in a line that starts as a comment does
    (
        r"^\s*([^\s#/*;<-].*)?(\*\*[A-Za-z][^*]*\*\*|`[^`]+`)",
        &[(MD, 1)],
    ),
    (r"^(={3,}|-{3,})\s*$", &[(MD, 1)]),
    // Rust
    (r"^\s*use (std|core|alloc|crate|self|super)::", &[(RUST, 3)]),
    (
        r"^\s*#!?\[(derive|cfg|allow|deny|warn|test|inline|must_use|doc|macro_export|non_exhaustive|repr|path|cfg_attr|tokio::|serde)",
        &[(RUST, 3)],
    ),
    (
        r"^\s*(pub(\([\w:]+\))?\s+)?((const|async|unsafe|extern( .C.)?)\s+)*fn \w+",
        &[(RUST, 3)],
    ),
    (r"^\s*impl(<.*>)?\s+[\w:<>]+", &[(RUST, 3)]),
    (
        r"^\s*pub(\([\w:]+\))? (struct|enum|trait|mod|use|type|const|static) ",
        &[(RUST, 3)],
    ),
    (r"\blet mut \w|&(mut )?self\b", &[(RUST, 3)]),
    (
        r"\b(println|format|vec|assert_eq|assert|panic|write|writeln|matches|unreachable|todo)!\(",
        &[(RUST, 3)],
    ),
    (
        r"-> (Self|Result<|Option<|Vec<|bool|usize|u8|u32|u64|i32|i64|&)",
        &[(RUST, 2)],
    ),
    // Go
    (
        r"^package [a-z_][a-z0-9_]*\s*$",
        &[(GO, 2), (KOTLIN, 1), (SCALA, 1)],
    ),
    (r#"^import \($|^import (\w+ )?"[\w./-]+"$"#, &[(GO, 3)]),
    (r"^func \(", &[(GO, 3)]),
    (r"^func \w+\(", &[(GO, 2), (SWIFT, 1)]),
    (r"^\s+[\w, ]+ := ", &[(GO, 2)]),
    (r"\bif err != nil\b", &[(GO, 3)]),
    (r"^type \w+ (struct|interface) \{", &[(GO, 3)]),
    // Swift
    (
        r"^import (Foundation|UIKit|SwiftUI|XCTest|AppKit|Combine|Cocoa|Dispatch|[A-Z]\w*Kit)$",
        &[(SWIFT, 4)],
    ),
    (r"^@testable import ", &[(SWIFT, 4)]),
    (
        r"^\s*(@\w+(\(.*\))?\s+)*((public|private|fileprivate|internal|open|static|override|final|mutating|class)\s+)*func \w+.*\(",
        &[(SWIFT, 2)],
    ),
    (
        r"\bguard (let|var|!|\w).* else \{|^\s*(if|guard) (let|var) \w+ = ",
        &[(SWIFT, 3)],
    ),
    (
        r"^\s*(public |private |fileprivate |internal |open |final )*(class|struct|enum|protocol|extension|actor) \w+(<.*>)?: \w",
        &[(SWIFT, 3)],
    ),
    (
        r"^\s*(public |private |fileprivate |internal )*(extension|protocol) [A-Z]\w*",
        &[(SWIFT, 2)],
    ),
    (
        r"^\s*@(objc|IBOutlet|IBAction|Published|State|Binding|MainActor|available|discardableResult|escaping|ObservedObject|Environment)\b",
        &[(SWIFT, 3)],
    ),
    (
        r"^\s*((public|private|fileprivate|internal|static|weak|lazy|@\w+)\s+)*let \w+: [A-Z\[]",
        &[(SWIFT, 2)],
    ),
    // Kotlin
    (
        r"^\s*((public|private|internal|protected|override|open|abstract|suspend|inline|operator|infix|tailrec|actual|expect)\s+)*fun (<[^>]*> )?[\w.]+\(",
        &[(KOTLIN, 4)],
    ),
    (
        r"^\s*(data|sealed|annotation|value|inner) class \w+",
        &[(KOTLIN, 3)],
    ),
    (
        r"\bcompanion object\b|\blistOf\(|\bmapOf\(|\bmutableListOf\(|\bsetOf\(|\?\.let \{",
        &[(KOTLIN, 3)],
    ),
    (
        r"^\s*((private|override|internal|const|lateinit|protected|public)\s+)*val \w+",
        &[(KOTLIN, 1), (SCALA, 1)],
    ),
    (
        r"^package [a-z]\w*(\.\w+)+\s*$",
        &[(KOTLIN, 2), (SCALA, 2), (GROOVY, 1)],
    ),
    (
        r"^import [a-z]\w*(\.\w+)+(\.\*)?\s*$",
        &[(KOTLIN, 1), (SCALA, 1), (GROOVY, 1)],
    ),
    (
        r"^\s*(implementation|api|testImplementation|kapt|ksp|compileOnly|runtimeOnly|androidTestImplementation|classpath)\(",
        &[(KOTLIN, 3)],
    ),
    (r#"^\s*id\(""#, &[(KOTLIN, 2)]),
    (r"\bwhen \(.*\) \{|\bwhen \{", &[(KOTLIN, 2)]),
    // Scala
    (
        r"^\s*((private|protected|override|final|implicit|lazy|sealed|abstract)(\[\w+\])?\s+)*def \w+(\[.*\])?(\(.*\))*\s*:\s*[\w\[\]., ]+=",
        &[(SCALA, 4)],
    ),
    (
        r"^\s*case class \w+|\bimplicit\b|\bextends \w+(\[.*\])?( with \w+)+|\w match \{",
        &[(SCALA, 3)],
    ),
    (
        r"^\s*((sealed|abstract|private|final)\s+)*(object|trait) \w+",
        &[(SCALA, 2)],
    ),
    (r"^import [\w.]+\.(_|\{[^}]*\}|given)\s*$", &[(SCALA, 3)]),
    (r"^\s*def \w+.*\)\s*=\s*", &[(SCALA, 2)]),
    // Groovy: Gradle, Jenkins, scripts
    (r"^\s*def \w+\s*=\s*", &[(GROOVY, 2), (SCALA, 1)]),
    (
        r"^\s*((private|public|static)\s+)*def \w+\(.*\)\s*\{\s*$",
        &[(GROOVY, 3)],
    ),
    (
        r"^\s*(pipeline|stages|steps|agent|post|environment|parameters)\s*\{|^\s*(node|stage)\s*(\(.*\))?\s*\{",
        &[(GROOVY, 2)],
    ),
    (
        r#"\bprintln\s+["'$]|^\s*apply plugin:|^\s*@(Grab|CompileStatic|TypeChecked|Field)\b"#,
        &[(GROOVY, 3)],
    ),
    (
        r#"^\s*(implementation|api|testImplementation|compile|compileOnly|runtimeOnly|testCompile|classpath|annotationProcessor)\s+['"]"#,
        &[(GROOVY, 3)],
    ),
    (r"^\s*id '", &[(GROOVY, 2)]),
    (
        r"^\s*((private|public|protected|static|final)\s+)+([A-Z][\w<>\[\], ]*|int|long|boolean|double|float|byte|char|short|def) \w+ = .*[^;{,(\s]\s*$",
        &[(GROOVY, 3)],
    ),
    (r"\{\s*\w+(,\s*\w+)*\s*->", &[(GROOVY, 2), (KOTLIN, 1)]),
    // Gradle's, in either language; Kotlin's backticked plugin ids
    (
        r#"^\s*(dependencies|repositories|plugins|allprojects|subprojects|buildscript|android|sourceSets|configurations|publishing|dependencyResolutionManagement|pluginManagement)\s*\{|\bproject\(['"]:|\bmavenCentral\(\)|^\s*rootProject\.|\blibs\.(plugins|versions)\."#,
        &[(GROOVY, 1), (KOTLIN, 1)],
    ),
    (r"^\s*`[\w-]+`\s*$|^\s*`[\w-]+`\(", &[(KOTLIN, 3)]),
    // Java
    (r"^package [\w.]+;\s*$", &[(JAVA, 3), (GROOVY, 1)]),
    (
        r"^import (static )?[\w.]+(\.\*)?;\s*$",
        &[(JAVA, 3), (GROOVY, 1)],
    ),
    (
        r"^\s*(public|private|protected)( static)?( final)?( abstract)? (class|interface|enum|record|@interface) \w+",
        &[(JAVA, 2), (CSHARP, 2)],
    ),
    (
        r"@Override\b|\bSystem\.(out|err)\.print|\bthrows [A-Z]\w*",
        &[(JAVA, 3)],
    ),
    (
        r"^\s*(public|private|protected) (static )?(final )?[A-Z][\w<>\[\], ]* \w+\(.*\)\s*(throws [\w., ]+)?\s*\{?\s*$",
        &[(JAVA, 2), (CSHARP, 1)],
    ),
    (
        r"^\s*(private|public|protected) (static )?final ",
        &[(JAVA, 2)],
    ),
    // C#
    (r"^\s*using (static )?[A-Z]\w*(\.\w+)*;\s*$", &[(CSHARP, 4)]),
    (
        r"^\s*namespace [A-Z][\w.]*\s*[;{]?\s*$",
        &[(CSHARP, 2), (CFAM, 1)],
    ),
    (
        r"\{ ?get; ?((private |protected |internal )?(set|init); ?)?\}",
        &[(CSHARP, 4)],
    ),
    (
        r"^\s*\[(assembly: |Test\]|Fact\]|TestMethod|Serializable|HttpGet|HttpPost|Route\(|Required|JsonProperty|Theory|InlineData|SetUp|TestFixture|DllImport|Obsolete|Theory)",
        &[(CSHARP, 3)],
    ),
    (
        r"\b(public|private|internal|protected) (static |async |override |virtual |readonly |sealed |partial |abstract )*(void|string|int|bool|Task|var|object)\b",
        &[(CSHARP, 2), (JAVA, 1)],
    ),
    (
        r"^\s*#(region|endregion)\b|\bConsole\.Write|\basync Task\b|\binternal (sealed |static )?class\b|\bvar \w+ = new\b",
        &[(CSHARP, 3)],
    ),
    // the C family
    (r#"^\s*#\s*include\s*[<"]"#, &[(CFAM, 4)]),
    (
        r#"^#import\s*[<"]|^\s*@(interface|implementation|protocol|end)\b"#,
        &[(CFAM, 4)],
    ),
    (
        r"^#\s*define\s+[A-Za-z_]\w*|^#\s*(ifndef|ifdef|undef)\s+\w+\s*$|^#\s*pragma once",
        &[(CFAM, 3)],
    ),
    (r"^#\s*(endif|else|if|elif)\b", &[(CFAM, 1), (CSHARP, 1)]),
    (
        r"^(static |extern |inline |const |unsigned |signed |struct |enum )*(void|int|char|long|short|float|double|bool|size_t|u?int\d+_t|\w+_t)\s*\**\s*\w+\s*\(",
        &[(CFAM, 2)],
    ),
    (
        r"\bstd::\w|\bnullptr\b|\btemplate\s*<|\btypename\b|\bconstexpr\b",
        &[(CFAM, 3)],
    ),
    (
        r"\b(printf|fprintf|malloc|free|sizeof|memcpy|strlen)\(|\bNULL\b",
        &[(CFAM, 1)],
    ),
    // Python
    (
        r"^\s*(async )?def \w+\(.*\)\s*(->\s*[^:]+)?:\s*(#.*)?$",
        &[(PYTHON, 4)],
    ),
    (r"^\s*(async )?def \w+\(\s*$", &[(PYTHON, 2)]),
    (r"^\s*class \w+(\(.*\))?:\s*$", &[(PYTHON, 4)]),
    (r"^from [\w.]+ import ", &[(PYTHON, 4)]),
    (
        r"^import [a-z_]\w*(, [a-z_]\w*)*( as \w+)?\s*$",
        &[(PYTHON, 2)],
    ),
    (
        r"^\s*(elif .*|except.*|else|try|finally|with .*)\s*:\s*(#.*)?$",
        &[(PYTHON, 3)],
    ),
    (r"^\s*(if|for|while) .*:\s*(#.*)?$", &[(PYTHON, 2)]),
    (
        r"\bif __name__ ==|\bself\.\w+ = |\bNone\b|\bTrue\b|\bFalse\b",
        &[(PYTHON, 1)],
    ),
    (r#"^\s*(r|f|b)?""""#, &[(PYTHON, 1)]),
    // Ruby
    (r#"^\s*require(_relative)? ['"]"#, &[(RUBY, 3), (LUA, 1)]),
    (r"^\s*def (self\.)?\w+[?!=]?(\(.*\))?\s*$", &[(RUBY, 3)]),
    (r"^\s*end\s*$", &[(RUBY, 1), (LUA, 1), (ELIXIR, 1)]),
    (
        r"^\s*(class|module) [A-Z]\w*(::[A-Z]\w*)*( < [A-Z][\w:]*)?\s*$",
        &[(RUBY, 3)],
    ),
    (
        r"\bdo \|[\w, *&]+\|\s*$|\{ ?\|\w+(, ?\w+)*\| |\battr_(reader|writer|accessor)\b|^# frozen_string_literal:",
        &[(RUBY, 3)],
    ),
    (r"\belsif\b", &[(RUBY, 2), (PERL, 1)]),
    (
        r"\bputs |\.freeze\b|\.each do\b|^\s*private\s*$",
        &[(RUBY, 1)],
    ),
    // Elixir
    (r"^\s*defmodule [A-Z][\w.]* do\s*$", &[(ELIXIR, 6)]),
    (
        r"^\s*(defp|defmacrop?|defguardp?|defdelegate|defstruct|defimpl|defprotocol|defexception)\b",
        &[(ELIXIR, 3)],
    ),
    (
        r"^\s*def \w+[?!]?(\(.*\))?( when .*)?,? do:?",
        &[(ELIXIR, 3)],
    ),
    (
        r"^\s*\|> |^\s*@(moduledoc|doc|spec|impl|type|typedoc|callback|behaviour)\b|\{:ok, |\bfn \w+(, \w+)* ->",
        &[(ELIXIR, 3)],
    ),
    (
        r"^\s*alias [A-Z][\w.]*(, as: \w+)?\s*$|^\s*use [A-Z][\w.]*(,.*)?\s*$|\bIO\.puts\b|\bMix\.\w|\bExUnit\b",
        &[(ELIXIR, 3)],
    ),
    // Lua
    (r"^\s*local function \w+", &[(LUA, 4)]),
    (r"^\s*local \w+(\s*,\s*\w+)* = ", &[(LUA, 2)]),
    (r"^\s*function \w+[.:]\w+\s*\(", &[(LUA, 3)]),
    (r"^\s*function \w+\s*\(", &[(LUA, 1), (JS, 1)]),
    (r"^\s*(if|elseif) [^;]* then\s*$", &[(LUA, 3)]),
    (r"~=|^\s*--\[\[|--\]\]|\bipairs\(|\bpairs\(", &[(LUA, 3)]),
    (r"^\s+[A-Za-z_]\w* = [^;=]*,\s*$", &[(LUA, 1)]),
    (r"^\s*--(\s|$|\[)", &[(LUA, 1), (SQL, 1), (HASKELL, 1)]),
    // Perl
    (
        r"^\s*use (strict|warnings|utf8|v?5[\d.]*|lib|constant|vars|parent|base|feature|POSIX|Carp|Exporter|[A-Z]\w*(::\w+)+)\b.*;",
        &[(PERL, 4)],
    ),
    (
        r"^\s*my [\$@%(]|^\s*sub \w+\s*(\{|$)|^\s*package \w+::[\w:]+;|^=(head\d|pod|cut|item|over|back)\b",
        &[(PERL, 3)],
    ),
    (r"\$\w+->\{|\$_\b|=~ [ms]?/|\bqw\(", &[(PERL, 2)]),
    // bash
    (r"^\s*(fi|esac|done)\s*(;.*|\|.*|>.*)?$", &[(BASH, 3)]),
    (
        r"^\s*(if|elif|while|until) \[\[? |; (then|do)\s*$|^\s*case .* in\s*$|^\s*set -[euxo]",
        &[(BASH, 3)],
    ),
    (
        r"^(function )?[\w-]+\s*\(\)\s*\{\s*$|^function [\w-]+\s*(\(\)\s*)?\{?\s*$",
        &[(BASH, 2)],
    ),
    (
        r"^\s*(export|readonly|declare|typeset|local) (-\w+ )?\w+=",
        &[(BASH, 3)],
    ),
    (
        r#"\$\{\w+[:#%/]|\|\| (exit|return)|2>&1|>\s*/dev/null|"\$[@*]"#,
        &[(BASH, 2)],
    ),
    (
        r"^\s*(echo|source|\.|cd|mkdir|rm|exec|exit|shift) ",
        &[(BASH, 1)],
    ),
    // PowerShell
    (r"^\s*(function|filter) [A-Z][a-z]+-[A-Z]\w*", &[(PS, 4)]),
    (
        r"\b(Get|Set|New|Remove|Write|Import|Export|Invoke|Add|Test|Start|Stop|Out|Select|Where|ForEach|Join|Split|ConvertTo|ConvertFrom|Format|Read|Clear|Copy|Move|Rename|Resolve|Register|Update|Install|Enable|Disable|Wait|Push|Pop|Measure|Sort|Group|Compare|Show|Expand|Compress)-[A-Z][a-zA-Z]+\b",
        &[(PS, 3)],
    ),
    (
        r"(?i)^\s*\[CmdletBinding|\[Parameter\(|^\s*param\s*\(|^\s*#requires\b|<#|#>",
        &[(PS, 3)],
    ),
    (
        r"(?i)\$(PSScriptRoot|PSBoundParameters|true|false|null|env:|_\.)|\[(string|int|bool|switch|hashtable|pscustomobject)\]",
        &[(PS, 3)],
    ),
    (
        r"\$[\w.]+\)?\s+-(like|notlike|match|notmatch|contains|notin|replace|join|split|f)\s",
        &[(PS, 2)],
    ),
    (
        r#"\$[\w.]+"?\)?\s+-(eq|ne|gt|lt|ge|le)\s"#,
        &[(PS, 1), (BASH, 1)],
    ),
    // SQL
    (
        r"(?i)^\s*(create\s+(or\s+replace\s+)?(temp(orary)?\s+)?(table|view|index|unique\s+index|function|procedure|trigger|schema|database|sequence|type|extension|materialized\s+view)|insert\s+into|alter\s+(table|database|sequence)|drop\s+(table|view|index|function|database|schema)|grant\s+\w+|delete\s+from|truncate\s+table|update\s+\w+\s+set)\b",
        &[(SQL, 4)],
    ),
    (
        r"^\s*(SELECT|FROM|WHERE|JOIN|LEFT JOIN|INNER JOIN|GROUP BY|ORDER BY|VALUES)\b|(?i)\bselect\s.+\sfrom\s+\w",
        &[(SQL, 2)],
    ),
    (
        r"(?i)\b(varchar|primary\s+key|foreign\s+key|not\s+null|auto_increment|bigint)\b",
        &[(SQL, 2)],
    ),
    // Haskell
    (r"^module [A-Z][\w.]*(\s*\(|\s+where|\s*$)", &[(HASKELL, 4)]),
    (r"^import qualified ", &[(HASKELL, 4)]),
    (
        r"^import [A-Z][\w.]*( as [A-Z]\w*)?( hiding)?( ?\(.*)?\s*$",
        &[(HASKELL, 1), (ELIXIR, 1)],
    ),
    (r"^[a-z_][\w']*\s+::\s+", &[(HASKELL, 4)]),
    (
        r"^(data|newtype|instance|deriving) |^class .* where\s*$|^\{-#|(^|\s)\{-[\s#]|\s-\}($|[^}])",
        &[(HASKELL, 3)],
    ),
    (r"\bwhere\s*$", &[(HASKELL, 2), (RUST, 1)]),
    // Nix
    (
        r"^\s*\{[\w\s,?@.]*\}:\s*$|^\s*\}:\s*$|^\s*\.\.\.\s*\}:",
        &[(NIX, 4)],
    ),
    (
        r"\b(mkDerivation|mkShell|fetchFromGitHub|fetchurl|buildInputs|nativeBuildInputs|stdenv|callPackage|mkIf|mkOption|mkEnableOption|nixpkgs)\b|\b(lib|pkgs)\.\w",
        &[(NIX, 3)],
    ),
    (r"^\s*inherit (\(.*\) )?[\w ]+;|\bwith [\w.]+;", &[(NIX, 3)]),
    (r"^\s*(let|in)\s*$", &[(NIX, 1), (HASKELL, 1)]),
    // Terraform and HCL
    (r#"^(resource|data) "[\w-]+" "[\w-]+" \{"#, &[(TF, 5)]),
    (
        r#"^(variable|output|provider|module) "[\w-]+" \{|^(locals|terraform|moved|removed|import|check) \{"#,
        &[(TF, 4)],
    ),
    (r#"^\s*[a-z_][\w-]* ("[^"]*" ?){1,2}\{\s*$"#, &[(TF, 3)]),
    (
        r"\$\{(var|local|module|data)\.|\b(var|local)\.[\w-]+",
        &[(TF, 2)],
    ),
    (r"^[a-z_][\w-]*\s*\{\s*$", &[(TF, 1)]),
    (
        r#"^\s+[a-z_][\w-]*\s+= ("[^"]*"|true|false|\d+|\[|\{)\s*$"#,
        &[(TF, 1)],
    ),
    // nginx
    (
        r"^\s*(server_name|proxy_pass|proxy_\w+|fastcgi_\w+|try_files|worker_processes|worker_connections|ssl_certificate\w*|ssl_\w+|add_header|client_max_body_size|keepalive_timeout|sendfile|default_type|error_page|access_log|error_log|uwsgi_\w+|gzip_\w+|limit_req\w*)\s",
        &[(NGINX, 3)],
    ),
    (
        r"^\s*location\s+[~=^@/]|^\s*listen\s+[\d\[]|^\s*(server|http|events|upstream \w+)\s*\{\s*$",
        &[(NGINX, 3)],
    ),
    // CMake
    (
        r"(?i)^\s*(cmake_minimum_required|cmake_policy|project|add_executable|add_library|target_\w+|find_package|add_subdirectory|set_target_properties|include_directories|add_definitions|add_custom_command|add_custom_target|add_dependencies|add_test|install|get_filename_component|configure_file|check_\w+|pkg_check_modules|endif|endforeach|endfunction|endmacro|endwhile|elseif)\s*\(",
        &[(CMAKE, 3)],
    ),
    (
        r"^\s*(set|unset|list|string|message|option|include|file|mark_as_advanced)\s*\(",
        &[(CMAKE, 2)],
    ),
    (r"\$\{CMAKE_\w+\}|\bCMAKE_[A-Z_]+", &[(CMAKE, 3)]),
    // Makefile
    (r"^\.PHONY\s*:|^define \w+|^endef\s*$", &[(MAKE, 4)]),
    (
        r"^(ifeq|ifneq|ifdef|ifndef)\s|^(-?include|sinclude) [^;]*$",
        &[(MAKE, 2)],
    ),
    (r"^endif\s*$", &[(MAKE, 1)]),
    (r"^[A-Za-z_][\w.]*\s*(::|:|\?|\+|!)=", &[(MAKE, 3)]),
    (
        r"^\t(@|-|\$\(|echo |cd |rm |mkdir |cp |install )",
        &[(MAKE, 2)],
    ),
    (
        r"\$\([A-Z_][A-Z0-9_]*\)|\$\((shell|wildcard|patsubst|subst|addprefix|addsuffix|foreach|call|filter|filter-out|dir|notdir|basename|eval|info|error|warning|strip|sort) |\$<|\$\^",
        &[(MAKE, 2)],
    ),
    (
        r"^[\w.%/$()-]+( [\w.%/$()-]+){0,2}\s*:([^=]|$)",
        &[(MAKE, 1)],
    ),
    // Dockerfile
    (
        r"^(RUN|COPY|ENTRYPOINT|WORKDIR|EXPOSE|CMD|HEALTHCHECK|ONBUILD|STOPSIGNAL|MAINTAINER|VOLUME)\s",
        &[(DOCKER, 3)],
    ),
    (
        r"(?i)^FROM\s+[\w./:${}@-]+(\s+AS\s+[\w-]+)?\s*$",
        &[(DOCKER, 2)],
    ),
    (r"^(ENV|ARG|LABEL|USER|ADD|SHELL)\s", &[(DOCKER, 1)]),
    // CSS and SCSS
    (
        r"^\s*-?[a-z]+(-[a-z]+)+\s*:\s*[^;{}]+;\s*$",
        &[(CSS, 2), (SCSS, 2)],
    ),
    (
        r"^\s*(color|margin|padding|width|height|display|position|top|left|right|bottom|border|background|content|float|clear|opacity|overflow|cursor|transition|transform|outline|visibility)\s*:\s*[^;{}]+;\s*$",
        &[(CSS, 2), (SCSS, 2)],
    ),
    (
        r"^\s*[.#][\w-]+[^;()]*\{\s*$|^\s*(a|body|html|div|span|p|h[1-6]|ul|ol|li|table|input|button|img|\*)([\s.:#\[>,][^;()]*)?\{\s*$",
        &[(CSS, 2), (SCSS, 2)],
    ),
    (
        r"^\s*@(media|font-face|keyframes|charset|supports|page|-webkit-keyframes|layer)\b",
        &[(CSS, 2), (SCSS, 1)],
    ),
    (
        r"!important\b|\b\d+(px|em|rem|vh|vw);",
        &[(CSS, 1), (SCSS, 1)],
    ),
    (
        r"^\s*[^{}]+\{(\s*[\w-]+\s*:[^;{}]+;)+",
        &[(CSS, 5), (SCSS, 1)],
    ),
    (r"^\s*\$[\w-]+\s*:", &[(SCSS, 4)]),
    (
        r"^\s*@(mixin|include|extend|use|forward|each|function|return|if|else)\b|#\{\$",
        &[(SCSS, 4)],
    ),
    (r"^\s*&[:.\-_\[ >+~]|&:\w", &[(SCSS, 1)]),
    // JavaScript, TypeScript, TSX
    (
        r#"^\s*import\s+(type\s+)?[\w*{}\s,$]+\s+from\s+['"]|^\s*import\s+['"]"#,
        &[(JS, 3), (TS, 3), (TSX, 3)],
    ),
    (
        r"^\s*export\s+(default|const|let|function|class|async|\{|\*)",
        &[(JS, 2), (TS, 2), (TSX, 2)],
    ),
    (
        r#"\bmodule\.exports\b|\bexports\.\w+ =|^\s*['"]use strict['"]"#,
        &[(JS, 5), (TS, 1)],
    ),
    (r#"\brequire\(['"]"#, &[(JS, 2), (TS, 1), (LUA, 1)]),
    (
        r"^\s*(const|let|var)\s+[\w${}\[\], ]+\s*=",
        &[(JS, 1), (TS, 1), (TSX, 1)],
    ),
    (
        r"^\s*const [\w$]+ = |^\s*(async )?function\s*\*?\s*[\w$]*\s*\([^)]*\)\s*\{",
        &[(JS, 2), (TS, 2), (TSX, 2)],
    ),
    (
        r"^\s*(let|var) [\w$]+ = .*;\s*$",
        &[(JS, 1), (TS, 1), (RUST, 1), (CSHARP, 1)],
    ),
    (
        r"\)\s*=>|^\s*\}\);?\s*$|\s(===|!==)\s",
        &[(JS, 2), (TS, 2), (TSX, 2)],
    ),
    (
        r"\bconsole\.\w+\(|\b(document|window)\.\w+",
        &[(JS, 2), (TS, 1), (TSX, 1)],
    ),
    (
        r"^\s*export (interface|type|enum|declare|abstract|namespace)\b|^\s*(declare|abstract class|namespace) \w",
        &[(TS, 3), (TSX, 3)],
    ),
    (
        r"^\s*(export\s+)?type\s+\w+(<.*>)?\s*=",
        &[(TS, 2), (TSX, 2)],
    ),
    (
        r"^\s*(export\s+)?interface\s+\w+(<.*>)?( extends [\w<>, .]+)?\s*\{",
        &[(TS, 2), (TSX, 2)],
    ),
    (
        r"[\w)\]]\??:\s*(string|number|boolean|any|unknown|void|never|undefined)(\[\])?\s*([;,)=|{>]|$)|\bimport type\b|\bas const\b|\breadonly \w+:|\bprivate (readonly )?\w+:",
        &[(TS, 3), (TSX, 3)],
    ),
    (
        r#"\bclassName=|^\s*</?[A-Z]\w*(\s+\w+=|\s*/?>\s*$|\s*$)|^\s*<>\s*$"#,
        &[(JS, 2), (TSX, 3)],
    ),
    (
        r#"^\s*import React\b|from ['"]react['"]|<\w+[^>]*/>\s*$|^\s*return \(\s*$"#,
        &[(JS, 1), (TSX, 2)],
    ),
    // Clojure and the Lisps
    (
        r"^\(ns [\w.-]+|^\s*\(:require \[|^\(defn-? |\((let|loop|doseq|for|binding|when-let|if-let) \[",
        &[(CLOJURE, 4)],
    ),
    (
        r"#\(|#\{|\(defprotocol|\(defrecord|\(defmulti",
        &[(CLOJURE, 2)],
    ),
    (r"^\(defproject ", &[(CLOJURE, 6)]),
    (
        r"^;;; [\w.-]+\.el\b|^;;;###autoload|\((defcustom|defgroup|defface|defvar-local|setq-local|define-minor-mode|define-derived-mode|use-package|add-hook|global-set-key|define-key|interactive|with-current-buffer|save-excursion|cl-defun|cl-loop)\b",
        &[(ELISP, 4)],
    ),
    (r"\((provide|require) '", &[(ELISP, 3)]),
    (r"\(setq ", &[(ELISP, 2), (LISP, 1)]),
    (
        r"^\(defun |^\(defmacro |^\(defvar ",
        &[(ELISP, 1), (LISP, 2)],
    ),
    (
        r"\((in-package|defpackage|defgeneric|defclass|defparameter|defstruct|defconstant|eval-when|declaim|define-condition|handler-case|multiple-value-bind|destructuring-bind|defsystem)[\s)]|\(format (t|nil) |#:\w",
        &[(LISP, 3)],
    ),
    (
        r"\((define-syntax|define-record-type|syntax-rules|let-values|call-with-current-continuation|call/cc|define-library|import \(|library \()",
        &[(SCHEME, 3), (RACKET, 2)],
    ),
    (r"^\(define \(?", &[(SCHEME, 2), (RACKET, 2)]),
    (r"#t\b|#f\b", &[(SCHEME, 1), (RACKET, 1)]),
    (r"^#lang ", &[(RACKET, 8)]),
    (
        r"\((require (racket|rackunit|2htdp)|module\+|define/contract|struct |for/list|for/fold|match-define)",
        &[(RACKET, 3)],
    ),
    // HTML and XML without a prolog
    (
        r"(?i)^\s*</?(html|head|body|div|span|p|a|script|style|meta|link|title|table|tr|td|th|ul|ol|li|form|input|button|img|br|hr|h[1-6]|section|header|footer|nav|main|article|iframe|label|select|option|textarea)\b[^>]*>",
        &[(HTML, 2)],
    ),
    (r"\bxmlns(:\w+)?=", &[(XML, 4)]),
    (
        r#"^\s*</?[\w:.-]+(\s+[\w:.-]+=("[^"]*"|'[^']*'))*\s*/?>\s*$"#,
        &[(XML, 1), (HTML, 1)],
    ),
];

/// The signals as one set, matched on bytes with ASCII classes (`\w`, `\b`, `\s`): Unicode's
/// make the lazy DFA give up on a set this size, and a line's signals are ASCII anyway.
static SET: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSetBuilder::new(SIGNALS.iter().map(|(re, _)| re))
        .unicode(false)
        .dfa_size_limit(32 << 20)
        .build()
        .unwrap()
});

/// The candidate `lines` show most, or `None` when none reaches [`MIN_SCORE`].
pub(super) fn best(lines: &[&str]) -> Option<&'static str> {
    let scores = scores(lines, |_, _| {});
    // the first of the highest: `max_by_key` keeps the last
    let top = scores.iter().map(|(_, s)| *s).max()?;
    (top >= MIN_SCORE)
        .then(|| scores.iter().find(|(_, s)| *s == top).map(|(l, _)| *l))
        .flatten()
}

/// Each candidate's total over `lines`, in [`ORDER`]; `hit` sees each signal that matched, by
/// its regex and line.
pub(super) fn scores(
    lines: &[&str],
    mut hit: impl FnMut(&'static str, &str),
) -> Vec<(&'static str, u32)> {
    let matched: Vec<_> = lines
        .iter()
        .map(|l| SET.matches(cut(l).as_bytes()))
        .collect();
    let only = |i: usize, lang: &str| SIGNALS[i].1.iter().all(|(l, _)| *l == lang);
    // Nix's `''` strings hold shell: once Nix's own signals show, the lines inside count for
    // Nix alone (only then: a Python line can end `= ''` too)
    let nix = matched.iter().any(|m| {
        m.iter()
            .any(|i| SIGNALS[i].1.iter().any(|&(l, w)| l == NIX && w >= 3))
    });
    let mut counts = vec![0u32; SIGNALS.len()];
    let (mut fenced, mut nix_string) = (false, false);
    for (line, m) in lines.iter().zip(&matched) {
        let line = cut(line);
        let trimmed = line.trim();
        if nix_string && trimmed.starts_with("''") && !trimmed.starts_with("'''") {
            nix_string = false;
        }
        for i in m {
            let counted = if fenced {
                only(i, MD)
            } else if nix_string {
                only(i, NIX)
            } else {
                true
            };
            if counted {
                counts[i] += 1;
                hit(SIGNALS[i].0, line);
            }
        }
        if nix && trimmed.ends_with("''") && !trimmed.ends_with("'''") && trimmed != "''" {
            nix_string = !trimmed.starts_with("''");
        }
        let fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if fence && line.len() - line.trim_start().len() <= 3 {
            fenced = !fenced;
        }
    }
    let mut scores: Vec<(&str, u32)> = ORDER.iter().map(|&l| (l, 0)).collect();
    for (i, n) in counts.iter().enumerate() {
        if *n == 0 {
            continue;
        }
        for &(lang, w) in SIGNALS[i].1 {
            if let Some(s) = scores.iter_mut().find(|(l, _)| *l == lang) {
                s.1 += u32::from(w) * (*n).min(CAP);
            }
        }
    }
    scores
}

/// The line's first [`LINE`] bytes, cut at a character.
fn cut(line: &str) -> &str {
    if line.len() <= LINE {
        return line;
    }
    let mut end = LINE;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_candidate_is_ordered_and_named() {
        for (re, langs) in SIGNALS {
            for (lang, _) in *langs {
                assert!(ORDER.contains(lang), "{lang} of {re} isn't in ORDER");
            }
        }
        // each a syntax's label, as `-l` and a file's name give it
        for &lang in ORDER.iter().filter(|&&l| l != MD) {
            assert_eq!(
                crate::filetype::FileType::named(lang),
                Some(crate::filetype::FileType::Text { lang: Some(lang) }),
                "{lang} isn't a syntax's label"
            );
        }
    }
}
