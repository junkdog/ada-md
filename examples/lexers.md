# ada-parse's lexers

One fenced snippet for each language ada highlights with a lexer of its own, so you can check them all in one place (`ada examples/lexers.md`).

## Bash

```bash
#!/usr/bin/env bash
# rebuild and print every snippet
set -euo pipefail

readonly root="${1:-$PWD}"
declare -A seen=()
count=0

render() {
    local file=$1 width=${2:-100}
    ada -p -w "$width" "$file" | tail -n +2
}

for f in "$root"/examples/*.md; do
    [[ -f $f && ! ${seen[$f]+x} ]] || continue
    seen[$f]=1
    (( count += 1 ))
    printf '%s: %d lines\n' "${f##*/}" "$(wc -l < "$f")"
done

cat <<EOF > /dev/stderr
rendered $count files at $(date +%H:%M), exit ${?}
EOF
case $count in 0) exit 1 ;; *) echo "done" ;; esac
```

## C

```c
#include <stdio.h>
#include <stdlib.h>

#define MAX_LINE 4096
#ifdef DEBUG
#  define LOG(...) fprintf(stderr, __VA_ARGS__)
#endif

/* a line read from a file */
typedef struct line {
    char text[MAX_LINE];
    size_t len;
} line_t;

static int count_words(const char *s) {
    int n = 0;
    for (; *s != '\0'; s++)
        if (*s == ' ' || *s == '\t') n++;
    return n + 1;
}

int main(void) {
    double ratio = 2.5e3; unsigned mask = 0xFFu;
    printf("%d words, ratio %.2f\n", count_words("a b c"), ratio);
    return mask > 0 ? EXIT_SUCCESS : EXIT_FAILURE;
}
```

## C#

```cs
using System;
using System.Linq;

namespace Ada.Demo;

/// <summary>A note with a title.</summary>
[Serializable]
public record Note(string Title, int Words = 0);

public static class Program
{
    private const double Ratio = 2.5e3;

    public static async Task<int> Main(string[] args)
    {
        var notes = new List<Note> { new("ada", 1_000), new("bat") };
        foreach (var n in notes.Where(n => n.Words > 0))
        {
            Console.WriteLine($"{n.Title}: {n.Words / Ratio:F2}\t");
        }
        string path = @"C:\notes\ada.md";
        int? maybe = null;
        await Task.Delay(0x10);
        return maybe ?? 0; // done
    }
}
```

## C++

```cpp
#include <iostream>
#include <vector>
#define SQUARE(x) ((x) * (x))

namespace ada {

/// A span over a line.
template <typename T>
class Span final : public Base {
public:
    explicit Span(T start, T end) noexcept : start_{start}, end_{end} {}
    [[nodiscard]] constexpr T len() const { return end_ - start_; }
    virtual ~Span() = default;

private:
    T start_, end_;
};

}  // namespace ada

int main(int argc, char** argv) {
    std::vector<ada::Span<int>> spans{{0, 4}, {4, 9}};
    auto ratio = 2.5 * 1e3 + 0x1Fu + 'a';
    for (const auto& s : spans) std::cout << "len: " << s.len() << '\n';
    const char* raw = R"(raw "string")";
    return argc > 1 ? EXIT_SUCCESS : static_cast<int>(ratio);
}
```

## Clojure

```clj
(ns ada.demo
  (:require [clojure.string :as str])
  (:import (java.time Instant)))

(def ^:private ratio 2.5)
(def ^:dynamic *limit* 0x10)

(defprotocol Counted (word-count [this]))

(defrecord Note [title words]
  Counted
  (word-count [_] words))

(defn count-words
  "Counts the words in `text`."
  [text & {:keys [limit] :or {limit *limit*}}]
  (min limit (count (re-seq #"\w+" text))))

(defmacro unless [test & body]
  `(if (not ~test) (do ~@body)))

(if-some [[_ first-name last-name] (re-matches #"(\w+)\s(\w+)" "Ada Lovelace")]
  (println first-name last-name)          ;; matching case
  (println "Unparsable name"))            ;; nil case

(->> [1 2 3 nil true :kw \a 1/3 2.5M 1e3 36rZZ]
     (filter some?)
     (map #(vector % (type %)))
     (into {}))
#_(println "ignored form")
```

## Crontab

```crontab
# m h dom mon dow command
SHELL=/bin/bash
MAILTO=ops@example.com
*/15 * * * * /usr/local/bin/sync.sh > /dev/null 2>&1
30 2 1-7 * mon tar -czf "/backup/$(date +%F).tgz" /srv  # TODO: rotate
0 9-17/2 * Jan,Jul 1-5 curl -fsS https://example.com/ping
@reboot sleep 30 && /opt/app/start.sh
```

## CSS

```css
/* ada's site */
@import url("fonts.css");
@media (prefers-color-scheme: dark) and (min-width: 40em) {
  :root { --bg: #282828; --fg: rgb(235 219 178 / 0.9); }
}

body > main .note:not(.draft)::before,
a[href^="https://"]:hover {
  color: var(--fg, #ebdbb2);
  margin: 0 auto 2.5rem;
  font: italic 1.2em/1.5 "Iosevka", monospace !important;
  transition: opacity 250ms ease-in-out;
  content: "\201C";
}

@keyframes fade { from { opacity: 0 } to { opacity: 1e0 } }
```

## Diff

```diff
diff --git a/ada-parse/src/lisp/words.rs b/ada-parse/src/lisp/words.rs
index 4f60163..137610d 100644
--- a/ada-parse/src/lisp/words.rs
+++ b/ada-parse/src/lisp/words.rs
@@ -96,6 +96,10 @@ const SCHEME_HEAD: u32 = 31 << 18;
 /// A Scheme or Racket procedure of the standard libraries.
 pub(super) const S_BUILTIN: u32 = 1 << 23;
-const SCHEME_IDENT: u32 = 3 << 24;
+const SCHEME_IDENT: u32 = 3 << 24;
+/// Clojure's heads.
+pub(super) const C_CONDITIONAL: u32 = 1 << 26;
\ No newline at end of file
```

## Dockerfile

```dockerfile
# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.91
FROM rust:${RUST_VERSION}-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY --chown=1000:1000 . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release && strip target/release/ada

FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="ada" version="1.0"
ENV TERM=xterm-256color LANG=C.UTF-8
COPY --from=build /src/target/release/ada /usr/local/bin/ada
COPY <<EOF /etc/ada/config.toml
theme = "gruvbox-dark"
EOF
RUN <<EOF
if [ -x /usr/local/bin/ada ]; then
  echo "ada $(ada --version)" > /etc/motd
fi
EOF
EXPOSE 8080/tcp
HEALTHCHECK --interval=30s CMD ["ada", "--version"]
USER 1000
ENTRYPOINT ["ada"]
CMD ["--help"]
```

## DotENV

```dotenv
# local development
export ADA_THEME=gruvbox-dark
ADA_WIDTH=100
RATIO=2.5
DATABASE_URL="postgres://ada:secret@localhost:5432/notes?sslmode=disable"
GREETING='single quoted $NOT_EXPANDED'
HOME_NOTES="${HOME}/notes/$USER"
EMPTY=
MULTI="line one\nline two"   # trailing comment
```

## Emacs Lisp

```elisp
;;; ada-mode.el --- view markdown with ada  -*- lexical-binding: t; -*-

(defgroup ada nil "Viewing markdown with ada." :group 'tools)

(defcustom ada-width 100
  "Print width."
  :type 'integer)

(defvar ada--ratio 2.5)

(defun ada-print (file)
  "Print FILE with ada."
  (interactive "fFile: ")
  (let ((buf (get-buffer-create "*ada*")))
    (with-current-buffer buf
      (erase-buffer)
      (call-process "ada" nil t nil "-p" "-w" (number-to-string ada-width) file)
      (when (eq (char-after (point-min)) ?\n)
        (message "starts with %c and %c" ?a ?\C-x)))
    (pop-to-buffer buf)))

(define-key global-map (kbd "C-c a") #'ada-print)
(provide 'ada-mode)
```

## fstab

```fstab
# <file system>  <dir>   <type>  <options>          <dump> <pass>
UUID=ad252f43-ade0-4506-a583-d6d5cb53f5c2  /      ext4  rw,relatime        0 1
/dev/nvme0n1p1                             /boot  vfat  rw,fmask=0077      0 2
server:/export                             /nfs   nfs4  _netdev,x-systemd  0 0
```

## Git Attributes

```gitattributes
# line endings and diffs
* text=auto eol=lf
*.rs diff=rust
*.png binary
*.bin -text -diff
assets/*.bin filter=lfs diff=lfs merge=lfs -text
CHANGELOG.md merge=union
[attr]generated -diff linguist-generated=true
site/data/*.json generated
```

## Git Ignore

```gitignore
# build output
/target/
**/*.rs.bk
*.log
!important.log
fuzz/corpus/**
vault-docs
.DS_Store
\#escaped-hash.txt
doc/[a-z]*.html
```

## Git Link

No fence token reaches this lexer yet (`.git` shows unhighlighted); a worktree's or submodule's `.git` file does: `ada path/to/worktree/.git`.

```.git
gitdir: ../.git/worktrees/ada-md-secondary
```

## Git Mailmap

```mailmap
# canonical names
Adrian Papari <junkdog@angelhill.net>
Adrian Papari <junkdog@angelhill.net> <adrian@old-laptop.local>
Ada Lovelace <ada@example.com> Ada <ada@users.noreply.example.com>
<new@example.com> <old@example.com>
```

## Go

```go
// Package demo counts notes.
package demo

import (
	"fmt"
	"regexp"
)

var word = regexp.MustCompile(`\w+`)

type Note struct {
	Title string `json:"title"`
	Words int    `json:"words,omitempty"`
}

func (n *Note) Ratio() float64 { return float64(n.Words) / 2.5e3 }

func Count[T comparable](xs []T, x T) (n int) {
	for _, v := range xs {
		if v == x {
			n++
		}
	}
	return
}

func main() {
	ch := make(chan int, 0x10)
	go func() { defer close(ch); ch <- len(word.FindAllString("a b", -1)) }()
	fmt.Printf("%d words, %q\n", <-ch, 'é')
}
```

## HTML

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>ada &mdash; a markdown viewer</title>
  <!-- inline CSS and JavaScript on their own lexers -->
  <style>
    body { background: #282828; margin: 2.5rem; }
  </style>
</head>
<body class="dark" data-version='1.0'>
  <h1 id="top">ada</h1>
  <p>Views <a href="https://example.com/?a=1&amp;b=2">markdown</a> &amp; code.</p>
  <img src="ada.jpg" alt="ada the cat" width=320>
  <script type="module">
    const n = document.querySelectorAll("p").length;
    console.log(`paragraphs: ${n}`);
  </script>
</body>
</html>
```

## INI

```ini
; ada's desktop entry and a few settings
[Desktop Entry]
Name=ada
Exec=ada %f
Terminal=true
MimeType=text/markdown;text/x-markdown;

[settings]
width = 100
ratio = 2.5
theme = "gruvbox-dark"   # inline comment
path = C:\Users\ada\notes
enabled = yes
```

## Java

```java
package ada.demo;

import java.util.*;
import java.util.regex.Pattern;

/** Counts the notes. */
@SuppressWarnings("unchecked")
public final class Counter<T extends Comparable<T>> implements AutoCloseable {
    private static final Pattern WORD = Pattern.compile("\\w+");
    private final Map<String, Integer> counts = new HashMap<>();
    private double ratio = 2.5e3, small = .5f;
    private long mask = 0xFF_FFL;

    @Override
    public void close() throws Exception {
        for (var e : counts.entrySet()) {
            if (e.getValue() > 10 && !e.getKey().isEmpty()) {
                System.out.printf("%s: %d%n", e.getKey(), e.getValue());
            }
        }
        String block = """
            text block "quoted" \t
            """;
        char c = '\n';
        Runnable r = () -> counts.clear(); /* lambda */
    }
}
```

## Java Properties

```properties
# messages for the viewer
! another comment style
app.name = ada
app.version=1.0.0
app.ratio: 2.5
greeting = Hello, {0}! You have {1,number} new notes.
path.windows = C:\\Users\\ada
unicode = caf\u00e9
long.value = first line \
    continued line
key\ with\ spaces = value
empty.value =
```

## JavaScript

```js
// count the words in notes
import { readFile } from "node:fs/promises";

const WORD = /\w+/gu;
const ratio = 2.5e3, mask = 0xff, big = 10n;

/** A note with a title. */
export class Note extends Base {
  #words = 0;
  static from(title) { return new Note(title); }
  get ratio() { return this.#words / ratio; }
}

export async function count(paths, { limit = 16 } = {}) {
  const notes = await Promise.all(paths.map(async (p) => {
    const text = await readFile(p, "utf8");
    return { title: p, words: text.match(WORD)?.length ?? 0 };
  }));
  for (const [i, n] of notes.entries()) {
    if (n.words > limit) console.log(`${i}: ${n.title}\t${n.words / ratio}`);
  }
  return notes.filter(({ words }) => words > 0); /* block */
}
```

## JSON

```json
{
  "name": "ada",
  "version": "1.0.0-rc.1",
  "tags": ["markdown", "viewer", "tui"],
  "ratio": 2.5,
  "big": 1e3,
  "negative": -0.25e-4,
  "escaped": "tab\there \"quoted\" \u00e9 \\ back",
  "enabled": true,
  "parent": null,
  "nested": { "depth": 3, "items": [{}, [], [1, 2, 3]] }
}
```

## Known Hosts

```known_hosts
# OpenSSH's known hosts
github.com,140.82.121.4 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl
[localhost]:2222,[::1]:2222 ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTY= dev box
|1|JfKTdBh7rNbXkVAQCRp4OQoPfmI=|USECr3SWf1JUPsms5AqfD5QfxkM= ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQC1
@cert-authority *.example.com ssh-dss AAAAB3NzaC1kc3MAAACBAP== ca
```

## Kotlin

```kotlin
package ada.demo

import kotlin.math.PI

/** A note with a title. */
@JvmInline
value class Title(val text: String)

data class Note(val title: Title, val words: Int = 0) {
    val ratio: Double get() = words / 2.5
}

sealed interface Shape { fun area(): Double }

fun <T : Comparable<T>> max(a: T, b: T): T = if (a > b) a else b

suspend fun main(args: Array<String>) {
    val notes = listOf(Note(Title("ada"), 1_000), Note(Title("bat")))
    val big = 1e3 + 0xFFL + PI
    for ((i, n) in notes.withIndex()) {
        when {
            n.words > 500 -> println("$i: ${n.title.text} is long\t(${n.ratio})")
            else -> println("""raw "$i" string""")
        }
    }
    notes.filter { it.words > 0 }.forEach(::println) // line comment
}
```

## Lisp (Common Lisp)

```lisp
;;;; count the words in notes
(defpackage :ada.demo (:use :cl) (:export #:count-words))
(in-package :ada.demo)

(defparameter *ratio* 2.5)
(defconstant +mask+ #xFF)

(defstruct note title (words 0 :type fixnum))

(defun count-words (text &optional (sep #\Space) &key (limit 16))
  "Counts the words in TEXT."
  (declare (type string text) (optimize (speed 3)))
  (let ((n 1))
    (loop for c across text
          when (char= c sep) do (incf n)
          finally (return (min n limit)))))

(defmacro with-note ((var title) &body body)
  `(let ((,var (make-note :title ,title)))
     ,@body))

#| block comment |#
(format t "~a: ~,2f~%" "ada" (/ (count-words "a b c") *ratio* 1e3))
```

## log

```log
2026-11-02 22:49:42.118 INFO  [main] ada::app: opened README.md (12 blocks, 3 code)
2026-11-02T22:49:42.207+01:00 DEBUG highlight: lexed 1,204 lines in 0.8 ms
2026-11-02 22:49:43,001 WARN  images: kitty probe timed out after 250ms, falling back to halfblocks
2026-11-02 22:49:44 ERROR print: failed to read /tmp/missing.md: No such file or directory (os error 2)
[2026-11-02 22:49:45] TRACE layout::table width=80 cols=4 rows=17
Nov  2 22:49:46 host kernel: [12345.678901] usb 1-2: new high-speed USB device number 5
E1102 22:49:47.000123   4021 server.go:88] connection reset by peer, retrying in 2s
```

## Lua

```lua
-- count the words in a note
local M = {}

--[[ a block
comment ]]
local ratio, big, mask = 2.5, 1e3, 0xFF

function M.count(text)
  local n = 0
  for _ in string.gmatch(text, "%w+") do
    n = n + 1
  end
  return n
end

local function greet(name, ...)
  local msg = ("Hello, %s!\t"):format(name or "ada")
  print(msg, select("#", ...), [[long "string"]])
  if n ~= nil and not done then goto continue end
  ::continue::
end

setmetatable(M, { __index = function(t, k) return rawget(t, k) end })
return M
```

## Makefile

```makefile
# build ada and its fuzz targets
CARGO ?= cargo
PROFILE := release
FEATURES = onig
SRC := $(wildcard src/*.rs) $(shell find ada-parse -name '*.rs')

.PHONY: all test clean

all: target/$(PROFILE)/ada

target/$(PROFILE)/ada: $(SRC) Cargo.toml
	$(CARGO) build --$(PROFILE) --features "$(FEATURES)"
	@echo "built $@ from $(words $^) files"

test: all
	$(CARGO) test --all-targets -- --nocapture

ifeq ($(OS),Windows_NT)
EXE := .exe
else
EXE :=
endif

clean:
	-rm -rf target/$(PROFILE) $(patsubst %.o,%.d,$(OBJS))
```

## PowerShell

```ps1
<#
  .SYNOPSIS
  Counts the words in notes.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Path,
    [int] $Limit = 0x10
)

$ratio = 2.5e3
$notes = Get-ChildItem -Path $Path -Filter *.md -Recurse

foreach ($note in $notes) {
    $words = (Get-Content $note.FullName -Raw) -split '\s+' | Measure-Object
    if ($words.Count -gt $Limit -and -not $note.Name.StartsWith('_')) {
        Write-Host "$($note.Name): $($words.Count / $ratio)`t" -ForegroundColor Green
    }
}

$here = @"
here-string $env:USERNAME
"@
try { throw [System.IO.IOException]::new("oops") } catch { Write-Error $_ } # comment
```

## Python

```python
#!/usr/bin/env python3
"""Count the words in notes."""
from __future__ import annotations

import re
from dataclasses import dataclass

WORD = re.compile(r"\w+")


@dataclass(frozen=True)
class Note:
    title: str
    words: int = 0

    @property
    def ratio(self) -> float:
        return self.words / 2.5e3


async def main(paths: list[str], *, limit: int = 0x10) -> None:
    notes = [Note(p, len(WORD.findall(open(p).read()))) for p in paths]
    for i, n in enumerate(notes):
        if n.words > limit and not n.title.startswith("_"):
            print(f"{i}: {n.title!r:>10} {n.ratio:.2f}\t", b"bytes\n")
    lambda x: x ** 2  # comment
```

## Racket

```rkt
#lang racket/base
;; count the words in notes
(require racket/string racket/list)
(provide count-words (struct-out note))

(struct note (title words) #:transparent)

(define ratio 2.5)
(define word-rx #px"\\w+")

(define (count-words text #:limit [limit 16])
  (min limit (length (regexp-match* word-rx text))))

(define/contract (ratio-of n)
  (-> note? real?)
  (/ (note-words n) ratio 1e3))

(for ([n (in-list (list (note "ada" 1000) (note "bat" 0)))]
      #:when (> (note-words n) 0))
  (printf "~a: ~a\n" (note-title n) (ratio-of n)))

(match (list 1 2) [(list a b) (+ a b)] [_ #f])
#;(displayln "commented out")
```

## Rust

```rust
//! Spans over a line.
use std::{borrow::Cow, fmt};

/// A highlighted stretch of a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span<'a> {
    text: &'a str,
    kind: u16,
}

impl<'a> Span<'a> {
    pub const MAX: usize = 0xffff;

    pub fn label(&self) -> Cow<'a, str> {
        match self.kind {
            0 => Cow::Borrowed(self.text),
            k @ 1..=9 => format!("{k}: {:>8}", self.text).into(),
            _ => r#"raw "string""#.into(),
        }
    }
}

fn ratio<T: Into<f64>>(x: T) -> f64 {
    let scale = 2.5_f64 * 1e3; /* block comment */
    x.into() / scale + b'\n' as f64 + '\u{e9}' as u32 as f64
}

macro_rules! span { ($t:expr) => { Span { text: $t, kind: 0 } }; }
```

## Scheme

```scheme
;; count the words in a string
(define-library (ada demo)
  (export count-words)
  (import (scheme base) (scheme char)))

(define ratio 2.5)

(define (count-words text)
  (let loop ((chars (string->list text)) (n 1))
    (cond ((null? chars) n)
          ((char-whitespace? (car chars)) (loop (cdr chars) (+ n 1)))
          (else (loop (cdr chars) n)))))

(define-record-type note
  (make-note title words)
  note?
  (title note-title)
  (words note-words))

#;(display "commented out datum")
(define-syntax swap!
  (syntax-rules () ((_ a b) (let ((tmp a)) (set! a b) (set! b tmp)))))

(when (and #t (not #f)) (display (/ (count-words "a b c") ratio 1e3)) (newline))
#| block comment |#
```

## SCSS

```scss
// ada's site, in Sass
@use "sass:math";
$base: 16px;
$palette: (bg: #282828, fg: #ebdbb2);

@mixin card($pad: 2.5rem) {
  padding: $pad;
  border-radius: math.div($base, 4);
}

.note {
  @include card;
  color: map-get($palette, fg);
  &:hover { color: darken(#ebdbb2, 10%); }
  &--draft { opacity: 0.5; }
  #{&}__title { font-size: $base * 1.5; }

  @each $name, $color in $palette {
    .theme-#{$name} { background: $color; }
  }
  @if $base > 12px { margin: 0 } @else { margin: 1px }
}
```

## SQL

```sql
-- notes and their words
CREATE TABLE IF NOT EXISTS notes (
    id      INTEGER PRIMARY KEY,
    title   VARCHAR(200) NOT NULL,
    words   INT DEFAULT 0,
    ratio   DECIMAL(5, 2) DEFAULT 2.5,
    created TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

/* the long ones */
SELECT n.title,
       COUNT(*) AS edits,
       AVG(n.words) / 1e3 AS kwords
FROM notes AS n
LEFT JOIN edits e ON e.note_id = n.id
WHERE n.title LIKE 'ada%' AND n.words > 0x10
GROUP BY n.title
HAVING COUNT(*) >= 2
ORDER BY kwords DESC
LIMIT 10;
```

## SSH Config

```ssh_config
# ~/.ssh/config
Host bastion *.corp !legacy.corp
  HostName 10.0.0.5
  User deploy
  Port 2222
  IdentityFile ~/.ssh/id_ed25519
  ForwardAgent yes
  ServerAliveInterval 60s
  Ciphers chacha20-poly1305@openssh.com,aes256-ctr,3des-cbc
  ProxyCommand ssh -W %h:%p jump.example.com
Match host *.lab exec "test -S ~/.ssh/agent.sock"
  IdentityAgent none
```

## SSHD Config

```sshd_config
# /etc/ssh/sshd_config
Port 22
ListenAddress 0.0.0.0
PermitRootLogin no
KexAlgorithms curve25519-sha256,diffie-hellman-group1-sha1
RekeyLimit 1G 1h
Subsystem sftp internal-sftp
Match Group sftp Address 10.0.0.0/8
  ChrootDirectory %h
  ForceCommand internal-sftp
```

## TOML

```toml
# ada's config
theme = "gruvbox-dark"
max_width = 100
ratio = 2.5
scale = 1e3
mask = 0xff_ff
born = 2026-11-02T22:49:42+01:00
escapes = "tab\there \u00e9"
literal = 'C:\no\escapes'
multi = """
first line
second line"""

[keys.viewer]
quit = ["q", "ctrl+c"]
inline = { search = "/", outline = "o" }

[[profiles]]
name = "print"
enabled = true
```

## TypeScript

```ts
// count the words in notes
import type { Stats } from "node:fs";

type Kind = "note" | "draft";

interface Note<T extends object = {}> {
  readonly title: string;
  words?: number;
  meta: T;
}

enum Level { Low = 1, High = 0x10 }

@sealed
export class Counter<T> implements Iterable<T> {
  private items: T[] = [];
  constructor(private readonly ratio: number = 2.5e3) {}
  *[Symbol.iterator](): Iterator<T> { yield* this.items; }
}

export function pick<T, K extends keyof T>(obj: T, ...keys: K[]): Pick<T, K> {
  const re = /^[a-z]+$/i;
  return keys.reduce((acc, k) => ({ ...acc, [k]: obj[k] }), {} as Pick<T, K>);
}

const n = { title: "ada" } satisfies Partial<Note>;
```

## TypeScriptReact

```tsx
import { useState, type FC } from "react";

interface Props {
  title: string;
  words?: number;
}

/** A note's card. */
export const Card: FC<Props> = ({ title, words = 0 }) => {
  const [open, setOpen] = useState<boolean>(false);
  const ratio = words / 2.5e3;
  return (
    <div className="card" data-words={words} onClick={() => setOpen(!open)}>
      <h2>{title}</h2>
      {open && <p style={{ opacity: 0.5 }}>ratio: {ratio.toFixed(2)}&nbsp;kw</p>}
      <Badge<number> value={0x10} />
      {/* a comment in JSX */}
    </div>
  );
};
```

## XML

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE feed SYSTEM "feed.dtd">
<!-- a feed of releases -->
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:ada="urn:ada:1">
  <title type="text">ada releases</title>
  <entry ada:id="42" draft='false'>
    <title>1.0.0-rc.1 &amp; friends</title>
    <updated>2026-11-02T22:49:42Z</updated>
    <content><![CDATA[<b>Lisp</b> highlighting & more]]></content>
    <link rel="alternate" href="https://example.com/ada?v=1&amp;x=2"/>
  </entry>
  <?render mode="print"?>
</feed>
```

## YAML

```yaml
%YAML 1.2
---
# CI for ada
name: ci
on:
  push:
    branches: [main, "release/*"]
env:
  RUSTFLAGS: -D warnings
  RATIO: 2.5
  MAX: 1e3
jobs:
  test: &defaults
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - run: |
          cargo test --all-targets
          echo "done: ${{ github.sha }}"
  lint:
    <<: *defaults
    enabled: !!bool true
    note: >-
      folded text
      on two lines
...
```
