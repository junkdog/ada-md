//! Compiles the two C++ shims: Lexilla's library parts and the five lexers measured (LexCPP for Java) (no catalogue,
//! so the other lexers aren't linked), and the KSyntaxHighlighting shim against the installed KF6.

use std::path::Path;

const LEXILLA: &str = "vendor/lexilla-rel-5-5-5";

fn main() {
    let lex = Path::new(LEXILLA);
    let mut lexilla = cc::Build::new();
    lexilla
        .cpp(true)
        .std("c++17")
        .opt_level(3)
        .define("NDEBUG", None)
        .include(lex.join("include"))
        // Lexilla's interfaces (ILexer.h, Scintilla.h) are Scintilla's: 5.6.7's headers
        .include("vendor/scintilla-5.6.7/scintilla/include")
        .include(lex.join("lexlib"))
        .include(lex.join("test"))
        .file(lex.join("test/TestDocument.cxx"))
        .file("shim/lexilla_shim.cxx");
    for f in std::fs::read_dir(lex.join("lexlib")).unwrap() {
        let p = f.unwrap().path();
        if p.extension().is_some_and(|e| e == "cxx") {
            lexilla.file(p);
        }
    }
    for name in ["LexJSON", "LexRust", "LexPython", "LexBash", "LexCPP"] {
        lexilla.file(lex.join(format!("lexers/{name}.cxx")));
    }
    lexilla.warnings(false).compile("lexilla_shim");

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .opt_level(3)
        .define("NDEBUG", None)
        .define("QT_NO_DEBUG", None)
        .flag("-fPIC")
        .include("/usr/include/qt6")
        .include("/usr/include/qt6/QtCore")
        // the Theme header reads QColor
        .include("/usr/include/qt6/QtGui")
        .include("/usr/lib/qt6/mkspecs/linux-g++")
        .include("/usr/include/KF6/KSyntaxHighlighting")
        .file("shim/ksh_shim.cxx")
        .compile("ksh_shim");
    println!("cargo:rustc-link-lib=dylib=KF6SyntaxHighlighting");
    println!("cargo:rustc-link-lib=dylib=Qt6Gui");
    println!("cargo:rustc-link-lib=dylib=Qt6Core");
    println!("cargo:rerun-if-changed=shim");
}
