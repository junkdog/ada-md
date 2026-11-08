// Lexilla behind a C ABI: a lexer by language, a document by file (Lexilla's own TestDocument,
// an in-memory IDocument like Scintilla's), and one call that styles a whole document from style
// 0, as Scintilla styles a file it has just opened.

#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

#include "ILexer.h"
#include "Scintilla.h"
#include "SciLexer.h"
#include "LexerModule.h"
#include "TestDocument.h"

using namespace Lexilla;

extern const LexerModule lmJSON;
extern const LexerModule lmRust;
extern const LexerModule lmPython;
extern const LexerModule lmBash;
extern const LexerModule lmCPP;

namespace {

// The keyword lists SciTE sets (its rust, python and bash .properties), so the lexers do the
// lookups a real editor's do.
const char *const rustKeywords =
    "as async await break const continue crate dyn else enum extern false fn for if impl in let "
    "loop match mod move mut pub ref return self Self static struct super trait true type union "
    "unsafe use where while abstract become box do final macro override priv try typeof unsized "
    "virtual yield";
const char *const rustTypes =
    "bool char f32 f64 i128 i16 i32 i64 i8 isize str u128 u16 u32 u64 u8 usize";
// SciTE's cpp.properties: keywordclass.java for Java's keywords, and the Javadoc tags LexCPP styles
// in a doc comment (its third list)
const char *const javaKeywords =
    "abstract assert boolean break byte case catch char class const continue default do double "
    "else enum extends final finally float for goto if implements import instanceof int interface "
    "long native new package private protected public return short static strictfp super switch "
    "synchronized this throw throws transient try var void volatile while";
const char *const javadocKeywords =
    "author code deprecated docRoot exception inheritDoc link linkplain literal param return see "
    "serial serialData serialField since throws value version";
const char *const pythonKeywords =
    "False None True and as assert async await break case class continue def del elif else except "
    "finally for from global if import in is lambda match nonlocal not or pass raise return try "
    "while with yield";
const char *const bashKeywords =
    "alias ar asa awk banner basename bash bc bdiff break bunzip2 bzip2 cal calendar case cat cc cd "
    "chmod cksum clear cmp col comm compress continue cp cpio crypt csplit ctags cut date dc dd "
    "declare deroff dev df diff diff3 dircmp dirname do done du echo ed egrep elif else env esac "
    "eval ex exec exit expand export expr false fc fgrep fi file find fmt fold for function "
    "functions getconf getopt getopts grep gres hash head help history iconv id if in integer jobs "
    "join kill local lc let line ln logname look ls m4 mail mailx make man mkdir more mt mv newgrp "
    "nl nm nohup ntps od pack paste patch pathchk pax pcat perl pg pr print printf ps pwd read "
    "readonly red return rev rm rmdir sed select set sh shift size sleep sort spell split start "
    "stop strings strip stty sum suspend sync tail tar tee test then time times touch tr trap true "
    "tsort tty type typeset ulimit umask unalias uname uncompress unexpand uniq unpack unset until "
    "uudecode uuencode vi vim vpax wait wc whence which while who wpaste wstart xargs zcat chgrp "
    "chown chroot dir dircolors factor groups hostid install link md5sum mkfifo mknod nice pinky "
    "printenv ptx readlink seq sha1sum shred stat su tac unlink users vdir whoami yes";

struct Doc {
    TestDocument doc;
    std::size_t len = 0;
};

} // namespace

extern "C" {

// 0 JSON, 1 Rust, 2 Python, 3 bash, 4 Java (LexCPP, as SciTE lexes Java)
void *lexilla_new(int lang) {
    Scintilla::ILexer5 *lexer = nullptr;
    switch (lang) {
    case 0:
        lexer = lmJSON.Create();
        lexer->WordListSet(0, "false null true");
        lexer->WordListSet(1, "@base @container @context @graph @id @index @language @list "
                              "@reverse @set @type @value @vocab");
        break;
    case 1:
        lexer = lmRust.Create();
        lexer->WordListSet(0, rustKeywords);
        lexer->WordListSet(1, rustTypes);
        break;
    case 2:
        lexer = lmPython.Create();
        lexer->WordListSet(0, pythonKeywords);
        break;
    case 3:
        lexer = lmBash.Create();
        lexer->WordListSet(0, bashKeywords);
        break;
    case 4:
        lexer = lmCPP.Create();
        lexer->WordListSet(0, javaKeywords);
        lexer->WordListSet(2, javadocKeywords);
        break;
    default:
        return nullptr;
    }
    return lexer;
}

void lexilla_free(void *lexer) {
    static_cast<Scintilla::ILexer5 *>(lexer)->Release();
}

void *lexilla_doc_new(const char *text, std::size_t len) {
    auto *d = new Doc;
    d->doc.Set(std::string_view(text, len));
    d->len = len;
    return d;
}

void lexilla_doc_free(void *doc) {
    delete static_cast<Doc *>(doc);
}

void lexilla_lex(void *lexer, void *doc) {
    auto *d = static_cast<Doc *>(doc);
    static_cast<Scintilla::ILexer5 *>(lexer)->Lex(0, static_cast<Sci_Position>(d->len), 0,
                                                    &d->doc);
}

// how many runs of one style the last lex left: the output's size, read outside the timing
std::size_t lexilla_runs(void *doc) {
    auto *d = static_cast<Doc *>(doc);
    std::size_t runs = 0;
    int prev = -1;
    for (std::size_t i = 0; i < d->len; i++) {
        const int s = static_cast<unsigned char>(d->doc.StyleAt(static_cast<Sci_Position>(i)));
        if (s != prev) {
            runs++;
            prev = s;
        }
    }
    return runs;
}

} // extern "C"
