//! ada-parse's phase-0 gate (ADR-40): printing a note whose fences are all plain or in languages
//! ada's own lexers cover never loads syntect's syntax set, nor does naming such a file; one
//! fence in another language does. One test, alone in its file: once loaded, the set stays
//! loaded for the process.

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

```text
plain
```

```
no language at all
```
";

fn print(note: &str) -> String {
    let path = Path::new("note.md");
    print::start(Some(note));
    let filetype = FileType::of(path, note);
    assert_eq!(filetype, FileType::Markdown);
    let doc = filetype.parse(path, note);
    let theme = Theme::builtin();
    let params = LayoutParams {
        width: 80,
        max_width: None,
        table_mode: TableMode::Auto,
        code_wrap: true,
        line_numbers: LineNumbers::Off,
        glyphs_gen: theme.glyphs.generation(),
        images: None,
    };
    print::render(&doc, &theme, filetype.params(params), Options::default())
}

#[test]
fn a_covered_only_note_prints_without_the_syntax_set() {
    let out = print(NOTE);
    assert!(out.contains("\x1b["), "highlighted: {out}");
    assert!(
        !highlight::syntaxes_loaded(),
        "a covered note loaded the set"
    );

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
    ] {
        let t = FileType::by_path(Path::new(name));
        assert!(
            matches!(t, Some(FileType::Text { lang: Some(_) })),
            "{name}"
        );
        assert!(!highlight::syntaxes_loaded(), "{name} loaded the set");
    }
    // a file of a name no table knows (`deploy`) still loads the set, to rule out a syntax that
    // claims the whole name; its first line alone doesn't
    for first in ["#!/bin/sh", "#!/usr/bin/env bash", "#!/usr/bin/make -f"] {
        assert!(highlight::first_line_language(first).is_some(), "{first}");
        assert!(!highlight::syntaxes_loaded(), "{first} loaded the set");
    }

    print(&format!("{NOTE}\n```ruby\nx = 1\n```\n"));
    assert!(highlight::syntaxes_loaded(), "a Ruby fence needs the set");
}
