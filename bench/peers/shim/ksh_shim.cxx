// KSyntaxHighlighting behind a C ABI. A document is a file's lines as QStrings, as Kate's buffer
// holds them (the UTF-8 to UTF-16 conversion is made once, outside the timing). A highlighter
// runs highlightLine over each line from the initial State and counts the format ranges it is
// handed, the counterpart of ada's spans.

#include <cstddef>
#include <cstdint>
#include <vector>

#include <QString>
#include <QStringView>

#include <KSyntaxHighlighting/AbstractHighlighter>
#include <KSyntaxHighlighting/Definition>
#include <KSyntaxHighlighting/Format>
#include <KSyntaxHighlighting/Repository>
#include <KSyntaxHighlighting/State>
#include <KSyntaxHighlighting/Theme>

namespace KSH = KSyntaxHighlighting;

namespace {

KSH::Repository *repository() {
    static KSH::Repository *repo = new KSH::Repository;
    return repo;
}

class Counter : public KSH::AbstractHighlighter {
public:
    std::uint64_t formats = 0;
    std::uint64_t covered = 0;

    void run(const std::vector<QString> &lines) {
        KSH::State state;
        for (const QString &line : lines) {
            state = highlightLine(QStringView(line), state);
        }
    }

protected:
    void applyFormat(int, int length, const KSH::Format &format) override {
        formats++;
        covered += static_cast<std::uint64_t>(length) + format.id();
    }
};

} // namespace

extern "C" {

// a definition by its name ("JSON", "Rust", "Python", "Bash"); null when there's none
void *ksh_new(const char *name) {
    const KSH::Definition def = repository()->definitionForName(QString::fromUtf8(name));
    if (!def.isValid()) {
        return nullptr;
    }
    auto *hl = new Counter;
    hl->setDefinition(def);
    hl->setTheme(repository()->defaultTheme(KSH::Repository::DarkTheme));
    return hl;
}

void ksh_free(void *hl) {
    delete static_cast<Counter *>(hl);
}

void *ksh_doc_new(const char *text, std::size_t len) {
    auto *lines = new std::vector<QString>;
    const QString all = QString::fromUtf8(text, static_cast<qsizetype>(len));
    for (const QStringView line : QStringView(all).split(u'\n')) {
        lines->push_back(line.toString());
    }
    // a file ending in a newline has no line after it
    if (!lines->empty() && lines->back().isEmpty()) {
        lines->pop_back();
    }
    return lines;
}

void ksh_doc_free(void *doc) {
    delete static_cast<std::vector<QString> *>(doc);
}

void ksh_lex(void *hl, void *doc) {
    static_cast<Counter *>(hl)->run(*static_cast<std::vector<QString> *>(doc));
}

// format ranges handed out since the last call
std::uint64_t ksh_take_formats(void *hl) {
    auto *c = static_cast<Counter *>(hl);
    const std::uint64_t n = c->formats;
    c->formats = 0;
    return n;
}

} // extern "C"
