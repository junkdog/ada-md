//! The languages ada's own lexers cover, and the others of bat's syntax set (ADR-40, ADR-43): a
//! note whose fences are in covered languages prints highlighted, and a file in one is named
//! by its name or its first line; a language of the set no lexer covers is named the same way,
//! and shows plain, as syntect left the binary in 1.1.

use std::path::Path;

use ada::filetype::FileType;
use ada::highlight;
use ada::layout::{LayoutParams, LineNumbers, TableMode};
use ada::print::{self, Options};
use ada::theme::Theme;

const NOTE: &str = "\
# Covered

```json
{\"name\": \"ada\", \"n\": [1, true, null]}
```

```zsh
for f in *.md; do echo \"$f\"; done
```

```Makefile
all: ada
\tcargo build --release
```

``` log
2026-10-31 12:00:01 INFO started pid=42
```

```diff
@@ -1 +1 @@ fn main
-old
+new
```

```ini
[core]
editor = vim ; the one
```

```toml
[package]
name = \"ada-md\" # why
deps = [1, { a = 2 }]
```

```xml
<?xml version=\"1.0\"?>
<root a='1'><!-- c --></root>
```

```yaml
name: ada
list: [1, two, {k: v}]
text: |
  block
```

```properties
key = value \\
  continued
```

```env
export NAME=\"ada\" # why
```

```gitignore
/target/
!keep.me
```

```dockerfile
FROM rust:1.88 AS build
RUN cargo build \\
    --release
```

```rust
#[derive(Debug)]
struct S<T>(Vec<T>);
fn main() { println!(\"{}\", r#\"raw\"#); }
```

```kotlin
package a.b
import x.Y as Z
data class P<T : Any>(val x: Int) : Q() { fun <R> List<T>.f(r: R): String = \"$x ${r}\" }
```

```java
/** A <b>doc</b> {@link Foo#bar(int)}. @param x the x */
@Override public <T> List<T> f(final int x) { return x -> new Foo<>(\"y\"); }
```

```cpp
#include <vector>
template <typename T> class Box { public: explicit Box(T v) : v_(v) {} T v_; };
int main() { auto s = R\"x(raw)x\"; return std::vector<int>{1, 2}.size(); }
```

```c
#include <stdio.h>
static int count(const char *s, size_t n) { return printf(\"%s %zu\\n\", s, n); }
```

```cs
/// <summary>A class.</summary>
[Serializable] public class P<T> { public string N { get; } = $\"x {y:N2}\"; }
```

```go
//go:generate stringer -type=Op
func (p *Pair[K, V]) Swap(a int) (r Pair[V, K]) { x := map[string][]byte{}; return fmt.Sprintf(\"%v {{.X}}\", x) }
```

```python
@app.route(\"/x\", methods=['GET'])
async def f(a, *b, c: int = 1) -> dict[str, int]:
    \"\"\"Doc.\"\"\"
    return {k: rf\"\\d{v!r:>4}\" for k, v in re.findall(r\"(?P<n>[a-z]+)\", \"SELECT 1\")}
```

```lua
local M = setmetatable({}, { __index = function(t, k) return rawget(t, k) end })
function M.run(a, ...) --[==[ long
]==] return string.format('%d\\n', #a) .. [[x]] end
```

```php
<?php #[Route('/x')] final class A extends B { public function f(?int $a = 1): string { return \"{$this->n} \" . preg_replace('/\\d+/', '', <<<SQL
SELECT 1
SQL); } } ?><b><?= $x ?></b>
```

```sql
CREATE TABLE t (id bigint PRIMARY KEY, n varchar(20) NOT NULL); -- note
SELECT a.b, count(*) FROM t WHERE n LIKE 'it''s' /* c */ GROUP BY a.b;
```

```powershell
function Get-X([Parameter(Mandatory)][string]$Name) { Get-ChildItem $env:TEMP | % { \"$($_.Name) `n\" } } # x
<# .SYNOPSIS #> $h = @{ a = 1; b = @'
'@ }
```

```js
export default class A extends B { get n() { return /ab+c/gi.test(`x ${this.y}`) ? <div a=\"1\">{n}</div> : 0x1F; } }
```

```javascript
const f = async (a, b = 2) => console.log(a / b);
```

```ts
export abstract class A<T extends B> implements C { private x?: Map<string, T[]> = new Map(); @dec m(): `a${T}` { return <T>this.y as const; } }
```

```tsx
export const L = <T,>({ items }: P<T>) => <ul className=\"l\">{items.map(i => <Item<T> key={i} />)} &amp;</ul>;
```

```typescript
enum E { A = 1 } namespace N { export type K = keyof typeof E; }
```

```css
@media (min-width: 40em) { a.btn:hover::before, x-el[href$=\".pdf\" i] { color: var(--c, #fc0) !important; margin: 0 -1.5em; } }
/* note */ @font-face { font-family: \"X\"; src: url(x.woff2) format(\"woff2\"); }
```

```scss
@use \"sass:math\"; $gap: math.div(10px, 2) !default; // note
.btn-#{$k} { &:hover { margin: -$gap auto; } @include mx($a: 1) { b: c; } }
```

```html
<!DOCTYPE html><p class=\"a\" style=\"color: red\" onclick=\"go(1)\">x &amp; y<br/></p><!-- c -->
<style>a:hover { color: #fc0 }</style><script>const x = `t ${1 < 2}`; // </script>
```

```lisp
(defun f (x &optional (y 2)) \"~a~%\" (loop for i in x collect (car i))) ; c
#| block |# (defstruct (s (:conc-name s-)) a) #'cl:car #\\Space #x1F 1/2 :key
```

```scheme
(define (f x) (let loop ([i 0]) (if (< i x) (loop (+ i 1)) #t))) #;(skip me) #\\a
```

```racket
#lang racket/base
(struct posn (x y) #:transparent) (for/list ([i 3]) (λ (v) v)) #rx\"a+\"
```

```clojure
(defn f [x & ys] {:pre [(pos? x)]} (str/join \",\" ys)) ; c
#{1 2} #(+ % 1) #\"a\\d+\" ^:private 1N 0x1F \\a ::k
```

```text
plain
```

```
no language at all
```
";

fn print(note: &str) -> String {
    let path = Path::new("note.md");
    print::start();
    let filetype = FileType::of(path, note);
    assert_eq!(filetype, FileType::Markdown);
    let doc = filetype.parse(path, note);
    let theme = Theme::builtin();
    let params = LayoutParams {
        width: 80,
        max_width: None,
        table_mode: TableMode::Auto,
        code_wrap: true,
        code_labels: true,
        bare: false,
        line_numbers: LineNumbers::Off,
        glyphs_gen: theme.glyphs.generation(),
        images: None,
    };
    print::render(&doc, &theme, filetype.params(params), Options::default())
}

#[test]
fn covered_languages_highlight_and_the_others_are_named() {
    let out = print(NOTE);
    assert!(out.contains("\x1b["), "highlighted: {out}");

    for name in [
        "Makefile",
        "build.sh",
        ".zshrc",
        "data.json",
        "rows.jsonl",
        "app.log",
        "config.mk",
        "fix.patch",
        "a.diff",
        "setup.cfg",
        ".editorconfig",
        "Cargo.toml",
        "Cargo.lock",
        "pom.xml",
        "feed.rss",
        "ci.yml",
        "config.yaml",
        "Dockerfile",
        "app.dockerfile",
        "app.properties",
        ".env",
        ".env.local",
        ".gitignore",
        ".gitattributes",
        ".mailmap",
        "main.rs",
        "Main.kt",
        "build.gradle.kts",
        "Main.java",
        "main.cpp",
        "widget.hpp",
        "main.c",
        "Program.cs",
        "build.csx",
        "main.go",
        "main.py",
        "stubs.pyi",
        "init.lua",
        "index.php",
        "view.phtml",
        "schema.sql",
        "profile.ps1",
        "Module.psm1",
        "app.js",
        "lib.mjs",
        "view.jsx",
        "config.cjs",
        "app.ts",
        "lib.mts",
        "conf.cts",
        "view.tsx",
        "site.css",
        "_variables.scss",
        "index.html",
        "page.htm",
        "init.el",
        "package.lisp",
        "boot.scm",
        "lib.sld",
        "init.scm",
        "build.ss",
        "main.rkt",
        "core.clj",
        "app.cljs",
        "deps.edn",
    ] {
        let t = FileType::by_path(Path::new(name));
        let lang = match t {
            Some(FileType::Text { lang: Some(lang) }) => lang,
            t => panic!("{name}: {t:?}"),
        };
        assert!(
            matches!(
                highlight::find_syntax(lang),
                Some(highlight::Syntax::Own(_))
            ),
            "{name}: {lang} has a lexer"
        );
    }
    // a name no syntax claims
    for name in ["deploy", "README", "notes.weird", "x.txt"] {
        assert_eq!(highlight::file_language(Path::new(name)), None, "{name}");
    }
    assert!(highlight::find_syntax("nosuchlanguage").is_none());
    // a first line that names a covered language, or none at all
    for first in [
        "#!/bin/sh",
        "#!/usr/bin/env bash",
        "#!/usr/bin/make -f",
        "#!/usr/bin/env python3",
        "<?xml version=\"1.0\"?>",
        "FROM rust:1.95",
    ] {
        let lang = highlight::first_line_language(first);
        let own = lang.and_then(highlight::find_syntax);
        assert!(matches!(own, Some(highlight::Syntax::Own(_))), "{first}");
    }
    for first in ["# ada", "<p align=\"center\">", "Some text.", "{", ""] {
        assert!(highlight::first_line_language(first).is_none(), "{first}");
    }

    // the set's other languages: named by file name, fence or first line, and plain
    let syntax = highlight::SyntaxTheme::new(&Theme::builtin().syntax);
    for (name, file, first) in [
        ("Haskell", "Main.hs", None),
        ("Perl", "x.pl", Some("#!/usr/bin/perl -w")),
        ("Elixir", "mix.exs", Some("#!/usr/bin/env elixir")),
        ("Git Log", "x.gitlog", Some("commit 0123456789abcdef")),
    ] {
        assert_eq!(highlight::file_language(Path::new(file)), Some(name));
        assert!(FileType::opens_in_ada(Path::new(file)), "{file}");
        assert!(
            matches!(highlight::find_syntax(name), Some(highlight::Syntax::Named(n)) if n == name)
        );
        if let Some(first) = first {
            assert_eq!(highlight::first_line_language(first), Some(name), "{first}");
        }
        let spans = highlight::highlight(1, name, "x = 1\ny = 2\n", &syntax);
        assert_eq!(spans.line_count(), 0, "{name} shows plain");
    }
}
