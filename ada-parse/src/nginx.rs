//! nginx, in place of bat's `nginx` (X-061): X-061's floor probe, a scanner of ada's own, not
//! bat's 54 contexts. One loop over a line's bytes in one of two places: `main`, where a word is
//! read once and looked up once (a directive's name, a block's keyword, a header), and a value
//! list (a directive's arguments to its `;`, an `if`'s condition to its `)`, a `map`'s entries
//! to its `}`), whose tokens bat finds where its patterns start (a variable, a string, a value
//! after a blank). A `*_by_lua_block {` hands its body to Lua's scanner ([`crate::lua::hosted`])
//! to the `}` that closes it. The state between lines is where the scan is (a directive's,
//! an `if`'s or a `map`'s values, a Lua block) and an open string, in `arg`'s high byte, with
//! Lua's own fields where a block is open; no block's nesting, which colors nothing.
//!
//! **bat's corners, followed:** a directive is a listed word with its list's scope (the first
//! list wins), `limit_req*` pushes nothing; an unknown word before a blank is a directive to its
//! `;` or the line's end, its `\s+` taking the blanks (so a value after it isn't one that starts
//! with a blank); a value after a blank (`[\t ]…(?=[\t ;])`) is an address, a number or a
//! constant only up to a blank or `;`, and `on`/`off` and the constants take the blank;
//! `location`'s, `upstream`'s and `map`'s headers to the line's first `{`; `rewrite`'s pattern;
//! a MIME type is a directive; `\.*\ ` runs to the line's last space and `^.*?$` to the next
//! `$`; a `#` is a comment only in `main` and a `map`.
//!
//! **Departures:** a `*_by_lua_block`'s body is Lua's at any indent, to the `}` nginx's block
//! reader ends it at (one where no table is open, past strings, comments and long brackets);
//! bat's embed matches only at a line's start (Oniguruma's `.*` prefix), leaves the directive's
//! name plain there, never pops a block that ends on its own line, and pops a Lua context at any
//! `}` that starts a line; indented, it reads the body as nginx directives, where a quote in the
//! Lua opens a string that can run on for the rest of the file. A `set_by_lua_block $x {` is
//! Lua's too. `\'` and `\"` in an argument are escaped quotes, as nginx reads them (bat opens a
//! string).
//!
//! **Kinds are leaves**, but for a directive's text: bat's directive contexts carry
//! `punctuation.definition.variable`, which colors text no other scope does (X-061's census: 20%
//! of the bytes, a color tuple of its own), and a token in them takes its own leaf's color, as
//! it does outside them; no `meta.*` scope colors anything. Lua's kinds are its own
//! ([`crate::lua::scopes`]), below [`NGINX`].

use crate::emit::{Runs, Sink, Spans};
use crate::words::{Words, text_len};
use crate::{Out, State};

/// The format's kinds are this plus their index in [`SCOPES`], then the directives' names
/// ([`NAME`]); Lua's are below it.
pub const NGINX: u16 = 0x100;

const COMMENT: u16 = NGINX;
const CONTEXT: u16 = NGINX + 1;
const CONTEXT_LOCATION: u16 = NGINX + 2;
const LOCATION_NAME: u16 = NGINX + 3;
const CONTROL: u16 = NGINX + 4;
const MAP_SEPARATOR: u16 = NGINX + 5;
/// A directive's text, `punctuation.definition.variable`'s.
const TEXT: u16 = NGINX + 6;
const VARIABLE: u16 = NGINX + 7;
const IP: u16 = NGINX + 8;
const CIDR: u16 = NGINX + 9;
const NUMBER: u16 = NGINX + 10;
const CONSTANT: u16 = NGINX + 11;
const REGEXP: u16 = NGINX + 12;
const DOUBLE: u16 = NGINX + 13;
const SINGLE: u16 = NGINX + 16;
// a string's, from its text
const ESCAPE: u16 = 1;
const STRING_VARIABLE: u16 = 2;
/// A directive's name, plus its scope's index in [`DIRECTIVE_SCOPES`].
const NAME: u16 = NGINX + 19;
const UNSUPPORTED: u16 = NAME + DIRECTIVE_SCOPES.len() as u16;

/// The scopes of the kinds from [`COMMENT`] to [`NAME`].
static SCOPES: [&str; 19] = [
    "comment.line.number-sign",
    "storage.type.context.nginx",
    "storage.type.context.location.nginx",
    "entity.name.context.location.nginx",
    "keyword.control.nginx",
    "punctuation.definition.map.nginx",
    "punctuation.definition.variable",
    "variable.other.nginx",
    "string.ipaddress.nginx",
    "constant.numeric.cidr.nginx",
    "constant.numeric.nginx",
    "constant.language.nginx",
    "string.regexp.nginx",
    "string.quoted.double.nginx",
    "string.quoted.double.nginx constant.character.escape.nginx",
    "string.quoted.double.nginx variable.other.nginx",
    "string.quoted.single.nginx",
    "string.quoted.single.nginx constant.character.escape.nginx",
    "string.quoted.single.nginx variable.other.nginx",
];

/// The keyword scopes of bat's directive lists, in the grammar's order.
static DIRECTIVE_SCOPES: [&str; 34] = [
    "keyword.directive.nginx",
    "keyword.directive.module.events.nginx",
    "keyword.directive.module.http.nginx",
    "keyword.directive.module.http.realip.nginx",
    "keyword.directive.module.http.access.nginx",
    "keyword.directive.module.http.auth_basic.nginx",
    "keyword.directive.module.http.autoindex.nginx",
    "keyword.directive.module.http.browser.nginx",
    "keyword.directive.module.http.charset.nginx",
    "keyword.directive.module.http.empty_gif.nginx",
    "keyword.directive.module.http.fastcgi.nginx",
    "keyword.directive.module.http.geo.nginx",
    "keyword.directive.module.http.gzip.nginx",
    "keyword.directive.module.http.gzip_static.nginx",
    "keyword.directive.module.http.headers.nginx",
    "keyword.directive.module.http.index.nginx",
    "keyword.directive.module.http.limit_req.nginx",
    "keyword.directive.module.http.limit_zone.nginx",
    "keyword.directive.module.http.limit_conn.nginx",
    "keyword.directive.module.http.log.nginx",
    "keyword.directive.module.http.map.nginx",
    "keyword.directive.module.http.memcached.nginx",
    "keyword.directive.module.http.proxy.nginx",
    "keyword.directive.module.http.referer.nginx",
    "keyword.directive.module.http.rewrite.nginx",
    "support.function.module.http.rewrite.nginx",
    "keyword.directive.module.http.scgi.nginx",
    "keyword.directive.module.http.split_clients.nginx",
    "keyword.directive.module.http.ssi.nginx",
    "keyword.directive.module.http.ssl.nginx",
    "keyword.directive.module.http.upstream.nginx",
    "keyword.directive.module.http.userid.nginx",
    "keyword.directive.module.http.uwsgi.nginx",
    "keyword.directive.module.http.stub_status.nginx",
];
const S_HTTP: u8 = 2;
const S_LIMIT_REQ: u8 = 16;
const S_REWRITE: u8 = 24;
const S_UPSTREAM: u8 = 30;
const S_STUB_STATUS: u8 = 33;

/// The scopes under `source.nginx` a kind carries, outermost first: the format's own, or Lua's.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind < NGINX {
        return crate::lua::scopes(kind);
    }
    let mut v = vec!["punctuation.definition.variable"];
    match kind {
        _ if kind < NAME => {
            v.clear();
            v.extend(SCOPES[usize::from(kind - NGINX)].split(' '));
        }
        _ if kind < UNSUPPORTED => v.push(DIRECTIVE_SCOPES[usize::from(kind - NAME)]),
        UNSUPPORTED => v.push("keyword.directive.module.unsupported.nginx"),
        _ => v.clear(),
    }
    v
}

// What a word is ([`INFO`]): a directive list's scope (its index), or one of these.
const I_NONE: u8 = 0xff;
/// `events`, `http`: a block's keyword before ` +{`.
const I_CONTEXT: u8 = 0x80;
/// `server`: a block's keyword before ` +{`, else the upstream list's directive.
const I_SERVER: u8 = 0x81;
/// `types`: a block's keyword before ` +{`.
const I_TYPES: u8 = 0x82;
const I_LOCATION: u8 = 0x83;
const I_UPSTREAM: u8 = 0x84;
const I_IF: u8 = 0x85;
const I_MAP: u8 = 0x86;
const I_REWRITE: u8 = 0x87;

/// bat's directive lists, each word with its list's scope (a word listed twice keeps its
/// first list's, the first pattern to match), then the words of `main`'s other patterns.
const WORD_LIST: [(&[u8], u8); 328] = [
    (b"daemon", 0),
    (b"env", 0),
    (b"debug_points", 0),
    (b"error_log", 0),
    (b"include", 0),
    (b"lock_file", 0),
    (b"master_process", 0),
    (b"pid", 0),
    (b"ssl_engine", 0),
    (b"timer_resolution", 0),
    (b"user", 0),
    (b"worker_cpu_affinity", 0),
    (b"worker_priority", 0),
    (b"worker_processes", 0),
    (b"worker_rlimit_core", 0),
    (b"worker_rlimit_nofile", 0),
    (b"worker_rlimit_sigpending", 0),
    (b"working_directory", 0),
    (b"accept_mutex", 1),
    (b"accept_mutex_delay", 1),
    (b"debug_connection", 1),
    (b"devpoll_changes", 1),
    (b"devpoll_events", 1),
    (b"epoll_events", 1),
    (b"kqueue_changes", 1),
    (b"kqueue_events", 1),
    (b"multi_accept", 1),
    (b"rtsig_signo", 1),
    (b"rtsig_overflow_events", 1),
    (b"rtsig_overflow_test", 1),
    (b"rtsig_overflow_threshold", 1),
    (b"use", 1),
    (b"worker_connections", 1),
    (b"aio", 2),
    (b"alias", 2),
    (b"chunked_transfer_encoding", 2),
    (b"client_body_in_file_only", 2),
    (b"client_body_in_single_buffer", 2),
    (b"client_body_buffer_size", 2),
    (b"client_body_temp_path", 2),
    (b"client_body_timeout", 2),
    (b"client_header_buffer_size", 2),
    (b"client_header_timeout", 2),
    (b"client_max_body_size", 2),
    (b"connection_pool_size", 2),
    (b"default_type", 2),
    (b"directio", 2),
    (b"error_page", 2),
    (b"if_modified_since", 2),
    (b"internal", 2),
    (b"keepalive_disable", 2),
    (b"keepalive_timeout", 2),
    (b"keepalive_requests", 2),
    (b"large_client_header_buffers", 2),
    (b"limit_except", 2),
    (b"limit_rate", 2),
    (b"limit_rate_after", 2),
    (b"lingering_close", 2),
    (b"lingering_time", 2),
    (b"lingering_timeout", 2),
    (b"listen", 2),
    (b"log_not_found", 2),
    (b"log_subrequest", 2),
    (b"msie_padding", 2),
    (b"msie_refresh", 2),
    (b"open_file_cache", 2),
    (b"open_file_cache_errors", 2),
    (b"open_file_cache_min_uses", 2),
    (b"open_file_cache_valid", 2),
    (b"optimize_server_names", 2),
    (b"port_in_redirect", 2),
    (b"post_action", 2),
    (b"recursive_error_pages", 2),
    (b"request_pool_size", 2),
    (b"reset_timedout_connection", 2),
    (b"resolver", 2),
    (b"resolver_timeout", 2),
    (b"root", 2),
    (b"satisfy", 2),
    (b"satisfy_any", 2),
    (b"send_timeout", 2),
    (b"sendfile", 2),
    (b"server_name", 2),
    (b"server_name_in_redirect", 2),
    (b"server_names_hash_max_size", 2),
    (b"server_names_hash_bucket_size", 2),
    (b"server_tokens", 2),
    (b"tcp_nodelay", 2),
    (b"tcp_nopush", 2),
    (b"try_files", 2),
    (b"underscores_in_hashes", 2),
    (b"variables_hash_bucket_size", 2),
    (b"variables_hash_max_size", 2),
    (b"types_hash_max_size", 2),
    (b"set_real_ip_from", 3),
    (b"real_ip_recursive", 3),
    (b"real_ip_header", 3),
    (b"allow", 4),
    (b"deny", 4),
    (b"auth_basic", 5),
    (b"auth_basic_user_file", 5),
    (b"autoindex", 6),
    (b"autoindex_exact_size", 6),
    (b"autoindex_localtime", 6),
    (b"ancient_browser", 7),
    (b"ancient_browser_value", 7),
    (b"modern_browser", 7),
    (b"modern_browser_value", 7),
    (b"charset", 8),
    (b"charset_map", 8),
    (b"override_charset", 8),
    (b"source_charset", 8),
    (b"empty_gif", 9),
    (b"fastcgi_bind", 10),
    (b"fastcgi_buffer_size", 10),
    (b"fastcgi_buffers", 10),
    (b"fastcgi_cache", 10),
    (b"fastcgi_cache_key", 10),
    (b"fastcgi_cache_path", 10),
    (b"fastcgi_cache_methods", 10),
    (b"fastcgi_cache_min_uses", 10),
    (b"fastcgi_cache_use_stale", 10),
    (b"fastcgi_cache_valid", 10),
    (b"fastcgi_connect_timeout", 10),
    (b"fastcgi_index", 10),
    (b"fastcgi_hide_header", 10),
    (b"fastcgi_ignore_client_abort", 10),
    (b"fastcgi_ignore_headers", 10),
    (b"fastcgi_intercept_errors", 10),
    (b"fastcgi_max_temp_file_size", 10),
    (b"fastcgi_no_cache", 10),
    (b"fastcgi_next_upstream", 10),
    (b"fastcgi_param", 10),
    (b"fastcgi_pass", 10),
    (b"fastcgi_pass_header", 10),
    (b"fastcgi_pass_request_body", 10),
    (b"fastcgi_pass_request_headers", 10),
    (b"fastcgi_read_timeout", 10),
    (b"fastcgi_redirect_errors", 10),
    (b"fastcgi_send_timeout", 10),
    (b"fastcgi_split_path_info", 10),
    (b"fastcgi_store", 10),
    (b"fastcgi_store_access", 10),
    (b"fastcgi_temp_path", 10),
    (b"geo", 11),
    (b"gzip", 12),
    (b"gzip_buffers", 12),
    (b"gzip_comp_level", 12),
    (b"gzip_disable", 12),
    (b"gzip_http_version", 12),
    (b"gzip_min_length", 12),
    (b"gzip_proxied", 12),
    (b"gzip_types", 12),
    (b"gzip_vary", 12),
    (b"gzip_static", 13),
    (b"add_header", 14),
    (b"expires", 14),
    (b"index", 15),
    (b"limit_req_log_level", 16),
    (b"limit_req_zone", 16),
    (b"limit_req", 16),
    (b"limit_zone", 17),
    (b"limit_conn", 17),
    (b"limit_conn_log_level", 17),
    (b"limit_conn_zone", 18),
    (b"access_log", 19),
    (b"log_format", 19),
    (b"open_log_file_cache", 19),
    (b"map_hash_max_size", 20),
    (b"map_hash_bucket_size", 20),
    (b"memcached_pass", 21),
    (b"memcached_connect_timeout", 21),
    (b"memcached_read_timeout", 21),
    (b"memcached_send_timeout", 21),
    (b"memcached_buffer_size", 21),
    (b"memcached_next_upstream", 21),
    (b"proxy_bind", 22),
    (b"proxy_buffer_size", 22),
    (b"proxy_buffering", 22),
    (b"proxy_buffers", 22),
    (b"proxy_busy_buffers_size", 22),
    (b"proxy_cache", 22),
    (b"proxy_cache_bypass", 22),
    (b"proxy_cache_key", 22),
    (b"proxy_cache_methods", 22),
    (b"proxy_cache_min_uses", 22),
    (b"proxy_cache_path", 22),
    (b"proxy_cache_use_stale", 22),
    (b"proxy_cache_valid", 22),
    (b"proxy_connect_timeout", 22),
    (b"proxy_headers_hash_bucket_size", 22),
    (b"proxy_headers_hash_max_size", 22),
    (b"proxy_hide_header", 22),
    (b"proxy_ignore_client_abort", 22),
    (b"proxy_ignore_headers", 22),
    (b"proxy_intercept_errors", 22),
    (b"proxy_max_temp_file_size", 22),
    (b"proxy_method", 22),
    (b"proxy_next_upstream", 22),
    (b"proxy_no_cache", 22),
    (b"proxy_pass", 22),
    (b"proxy_http_version", 22),
    (b"proxy_pass_header", 22),
    (b"proxy_pass_request_body", 22),
    (b"proxy_pass_request_headers", 22),
    (b"proxy_redirect", 22),
    (b"proxy_read_timeout", 22),
    (b"proxy_redirect_errors", 22),
    (b"proxy_send_lowat", 22),
    (b"proxy_send_timeout", 22),
    (b"proxy_set_body", 22),
    (b"proxy_set_header", 22),
    (b"proxy_ssl_session_reuse", 22),
    (b"proxy_store", 22),
    (b"proxy_store_access", 22),
    (b"proxy_temp_file_write_size", 22),
    (b"proxy_temp_path", 22),
    (b"proxy_upstream_fail_timeout", 22),
    (b"proxy_upstream_max_fails", 22),
    (b"valid_referers", 23),
    (b"rewrite_log", 24),
    (b"set", 24),
    (b"uninitialized_variable_warn", 24),
    (b"break", 25),
    (b"return", 25),
    (b"scgi_bind", 26),
    (b"scgi_buffer_size", 26),
    (b"scgi_buffering", 26),
    (b"scgi_buffers", 26),
    (b"scgi_busy_buffers_size", 26),
    (b"scgi_cache", 26),
    (b"scgi_cache_bypass", 26),
    (b"scgi_cache_key", 26),
    (b"scgi_cache_methods", 26),
    (b"scgi_cache_min_uses", 26),
    (b"scgi_cache_path", 26),
    (b"scgi_cache_use_stale", 26),
    (b"scgi_cache_valid", 26),
    (b"scgi_connect_timeout", 26),
    (b"scgi_hide_header", 26),
    (b"scgi_ignore_client_abort", 26),
    (b"scgi_ignore_headers", 26),
    (b"scgi_intercept_errors", 26),
    (b"scgi_max_temp_file_size", 26),
    (b"scgi_next_upstream", 26),
    (b"scgi_no_cache", 26),
    (b"scgi_param", 26),
    (b"scgi_pass", 26),
    (b"scgi_pass_header", 26),
    (b"scgi_pass_request_body", 26),
    (b"scgi_pass_request_headers", 26),
    (b"scgi_read_timeout", 26),
    (b"scgi_send_timeout", 26),
    (b"scgi_store", 26),
    (b"scgi_store_access", 26),
    (b"scgi_temp_file_write_size", 26),
    (b"scgi_temp_path", 26),
    (b"split_clients", 27),
    (b"ssi", 28),
    (b"ssi_silent_errors", 28),
    (b"ssi_types", 28),
    (b"ssi_value_length", 28),
    (b"ssl", 29),
    (b"ssl_certificate", 29),
    (b"ssl_certificate_key", 29),
    (b"ssl_client_certificate", 29),
    (b"ssl_dhparam", 29),
    (b"ssl_ciphers", 29),
    (b"ssl_crl", 29),
    (b"ssl_prefer_server_ciphers", 29),
    (b"ssl_protocols", 29),
    (b"ssl_verify_client", 29),
    (b"ssl_verify_depth", 29),
    (b"ssl_session_cache", 29),
    (b"ssl_session_timeout", 29),
    (b"ip_hash", 30),
    (b"server", 30),
    (b"userid", 31),
    (b"userid_domain", 31),
    (b"userid_expires", 31),
    (b"userid_name", 31),
    (b"userid_p3p", 31),
    (b"userid_path", 31),
    (b"userid_service", 31),
    (b"uwsgi_bind", 32),
    (b"uwsgi_buffer_size", 32),
    (b"uwsgi_buffering", 32),
    (b"uwsgi_buffers", 32),
    (b"uwsgi_busy_buffers_size", 32),
    (b"uwsgi_cache", 32),
    (b"uwsgi_cache_bypass", 32),
    (b"uwsgi_cache_key", 32),
    (b"uwsgi_cache_methods", 32),
    (b"uwsgi_cache_min_uses", 32),
    (b"uwsgi_cache_path", 32),
    (b"uwsgi_cache_use_stale", 32),
    (b"uwsgi_cache_valid", 32),
    (b"uwsgi_connect_timeout", 32),
    (b"uwsgi_hide_header", 32),
    (b"uwsgi_ignore_client_abort", 32),
    (b"uwsgi_ignore_headers", 32),
    (b"uwsgi_intercept_errors", 32),
    (b"uwsgi_max_temp_file_size", 32),
    (b"uwsgi_modifier1", 32),
    (b"uwsgi_modifier2", 32),
    (b"uwsgi_next_upstream", 32),
    (b"uwsgi_no_cache", 32),
    (b"uwsgi_param", 32),
    (b"uwsgi_pass", 32),
    (b"uwsgi_pass_header", 32),
    (b"uwsgi_pass_request_body", 32),
    (b"uwsgi_pass_request_headers", 32),
    (b"uwsgi_read_timeout", 32),
    (b"uwsgi_send_timeout", 32),
    (b"uwsgi_store", 32),
    (b"uwsgi_store_access", 32),
    (b"uwsgi_string", 32),
    (b"uwsgi_temp_file_write_size", 32),
    (b"uwsgi_temp_path", 32),
    (b"stub_status", 33),
    (b"events", I_CONTEXT),
    (b"http", I_CONTEXT),
    (b"types", I_TYPES),
    (b"location", I_LOCATION),
    (b"upstream", I_UPSTREAM),
    (b"if", I_IF),
    (b"map", I_MAP),
    (b"rewrite", I_REWRITE),
];
const N: usize = WORD_LIST.len();
const NAMES: [&[u8]; N] = {
    let mut t: [&[u8]; N] = [b""; N];
    let mut i = 0;
    while i < N {
        t[i] = WORD_LIST[i].0;
        i += 1;
    }
    t
};
static WORDS: Words<1024, N, { text_len(&NAMES) }> = Words::new(&NAMES);
/// A word's [`I_NONE`]… or scope by its id.
static INFO: [u8; N + 1] = {
    let mut t = [I_NONE; N + 1];
    let mut i = 0;
    while i < N {
        t[i + 1] = WORD_LIST[i].1;
        i += 1;
    }
    // `server` is the upstream list's, and a block's keyword
    t[WORDS.find(b"server") as usize] = I_SERVER;
    t
};

// A byte's action in `main` ([`MAIN`]).
const M_OTHER: u8 = 0;
const M_SPACE: u8 = 1;
const M_WORD: u8 = 2;
/// A byte past ASCII: a word's (a letter's, nearly always).
const M_HIGH: u8 = 3;
const M_HASH: u8 = 4;
static MAIN: [u8; 256] = {
    let mut t = [M_OTHER; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = match b {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => M_SPACE,
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' => M_WORD,
            0x80.. => M_HIGH,
            b'#' => M_HASH,
            _ => M_OTHER,
        };
        c += 1;
    }
    t
};

// A byte's action in a value list ([`VALUE`]).
const V_OTHER: u8 = 0;
const V_BLANK: u8 = 1;
const V_SEMI: u8 = 2;
const V_RPAREN: u8 = 3;
const V_LBRACE: u8 = 4;
const V_RBRACE: u8 = 5;
const V_HASH: u8 = 6;
const V_DOLLAR: u8 = 7;
const V_QUOTE: u8 = 8;
const V_TILDE: u8 = 9;
const V_BANG: u8 = 10;
const V_BACKSLASH: u8 = 11;
const V_CARET: u8 = 12;
static VALUE: [u8; 256] = {
    let mut t = [V_OTHER; 256];
    t[b' ' as usize] = V_BLANK;
    t[b'\t' as usize] = V_BLANK;
    t[b';' as usize] = V_SEMI;
    t[b')' as usize] = V_RPAREN;
    t[b'{' as usize] = V_LBRACE;
    t[b'}' as usize] = V_RBRACE;
    t[b'#' as usize] = V_HASH;
    t[b'$' as usize] = V_DOLLAR;
    t[b'"' as usize] = V_QUOTE;
    t[b'\'' as usize] = V_QUOTE;
    t[b'~' as usize] = V_TILDE;
    t[b'!' as usize] = V_BANG;
    t[b'\\' as usize] = V_BACKSLASH;
    t[b'^' as usize] = V_CARET;
    t
};

/// The length of the character past ASCII at `p` where it's a word's (`\w` as Oniguruma reads
/// it: a letter or a digit, not a mark like a byte-order mark or `：`), else 0.
#[cold]
#[inline(never)]
fn high_word(b: &[u8], p: usize) -> usize {
    let l = crate::prim::char_len(b[p]);
    match b.get(p..p + l).and_then(|c| std::str::from_utf8(c).ok()) {
        Some(c) if c.chars().all(char::is_alphanumeric) => l,
        _ => 0,
    }
}

/// Whether a word's character (`\w`) starts at `p`.
#[inline(always)]
fn word_at(b: &[u8], p: usize) -> bool {
    p < b.len() && (crate::prim::WORD[usize::from(b[p])] || b[p] >= 0x80 && high_word(b, p) > 0)
}

/// Where the word's characters (`\w`) from `p` end.
#[inline(always)]
fn word_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len() {
        if crate::prim::WORD[usize::from(b[p])] {
            p += 1;
        } else if b[p] >= 0x80 && high_word(b, p) > 0 {
            p += high_word(b, p);
        } else {
            break;
        }
    }
    p
}

/// `[\t ;]`, what ends a value after a blank.
static VALUE_END: [bool; 256] = crate::prim::stops(b"\t ;");

use crate::line_format::BLANK;

/// Past the spaces (` +`, not tabs) at `p`.
#[inline(always)]
fn spaces(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && b[p] == b' ' {
        p += 1;
    }
    p
}

/// Where `\$[A-Za-z0-9\_]+` at `p` ends, `p` where there's none.
#[inline(always)]
fn dollar_word(b: &[u8], p: usize) -> usize {
    if b.get(p) != Some(&b'$') {
        return p;
    }
    let mut e = p + 1;
    while e < b.len() && crate::prim::WORD[usize::from(b[e])] {
        e += 1;
    }
    if e == p + 1 { p } else { e }
}

// Where a line is ([`Scan::ctx`]).
const C_MAIN: u8 = 0;
/// A directive's arguments, to its `;`.
const C_DIRECTIVE: u8 = 1;
/// An unknown directive's arguments, to its `;` or the line's end.
const C_UNSUPPORTED: u8 = 2;
/// An `if`'s condition, to its `)`.
const C_IF: u8 = 3;
/// A `map`'s entries, to its `}`.
const C_MAP: u8 = 4;
/// A `*_by_lua_block`'s arguments, to its `{`.
const C_LUA_HEAD: u8 = 5;
/// A `*_by_lua_block`'s body, Lua's, to its `}`.
const C_LUA: u8 = 6;

// An open string ([`Scan::string`]).
const S_NONE: u8 = 0;
const S_DOUBLE: u8 = 1;
const S_SINGLE: u8 = 2;

/// Lex `b`, a line of nginx's config without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on::<_, false>(b, state, out, Spans::new());
}

/// [`line()`]'s lexemes written to `k`, a run sink.
#[inline(never)]
pub(crate) fn line_runs(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
    *k = line_on::<_, true>(b, state, out, *k);
}

#[inline(always)]
fn line_on<K: Sink, const SHARED: bool>(b: &[u8], state: &mut State, out: &mut Out, k: K) -> K {
    let host = (state.arg >> 8) as u8;
    let mut s = Scan {
        b,
        ctx: host & 7,
        string: host >> 3 & 3,
        meta: false,
        at: 0,
        space: usize::MAX,
        dollar: 0,
        brace: 0,
        st: state,
        k,
        out,
    };
    s.meta = meta(s.ctx);
    s.k.open(s.out, b.len() / 4 + 16);
    s.lex_line::<SHARED>();
    s.k.close(s.out);
    let host = u16::from(s.ctx | s.string << 3) << 8;
    if s.ctx != C_LUA {
        *s.st = State::default();
    }
    s.st.arg = s.st.arg & 0xff | host;
    s.k
}

/// Whether a context's text is a directive's ([`TEXT`]).
#[inline(always)]
fn meta(ctx: u8) -> bool {
    matches!(ctx, C_DIRECTIVE | C_UNSUPPORTED | C_LUA_HEAD)
}

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    /// Where the scan is ([`C_MAIN`]…).
    ctx: u8,
    /// A string open ([`S_NONE`]…).
    string: u8,
    /// Whether the text is a directive's ([`TEXT`]).
    meta: bool,
    /// Where the spans so far end: in a directive, its text from there to a token is
    /// `punctuation.definition.variable`'s.
    at: usize,
    /// Searches' answers kept for the line ([`Scan::last_space`], [`Scan::next_dollar`],
    /// [`Scan::next_brace`]): a construct repeated along a line costs one search, not one each.
    space: usize,
    dollar: usize,
    brace: usize,
    /// The state: Lua's in a Lua block, the host's byte in `arg`'s high byte.
    st: &'b mut State,
    k: K,
    out: &'b mut Out<'c>,
}

impl<K: Sink> Scan<'_, '_, K> {
    /// A write: one call on the run sink ([`Sink::merge_call`]), whose writer inlined at each
    /// of the scan's writes was +30 KB in ada.
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge_call(&mut self.out, s, e, kind);
    }

    /// The text to `e` is the directive's where the scan is in one.
    #[inline(always)]
    fn fill(&mut self, e: usize) {
        if self.meta && e > self.at {
            self.put(self.at, e, TEXT);
        }
        self.at = e;
    }

    /// `s..e` is `kind`, the text before it filled.
    #[inline(always)]
    fn tok(&mut self, s: usize, e: usize, kind: u16) {
        self.fill(s);
        self.put(s, e, kind);
        self.at = e;
    }

    #[inline(always)]
    fn enter(&mut self, ctx: u8, at: usize) {
        self.ctx = ctx;
        self.meta = meta(ctx);
        self.at = at;
    }

    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        let n = self.b.len();
        let mut p = 0;
        if self.string != S_NONE {
            p = self.string_rest(0);
        }
        while p < n {
            p = match self.ctx {
                C_MAIN => self.main(p),
                C_LUA => self.lua::<SHARED>(p),
                _ => self.values(p),
            };
        }
        if self.ctx != C_MAIN && self.ctx != C_LUA {
            self.fill(n);
            if self.ctx == C_UNSUPPORTED && self.string == S_NONE {
                self.enter(C_MAIN, n);
            }
        }
    }

    /// Lua from `p` to its block's `}` (plain), or the line's end. Out of line: Lua's set-up
    /// and state round trip once in ada, not at each caller.
    #[inline(never)]
    fn lua<const SHARED: bool>(&mut self, p: usize) -> usize {
        match crate::lua::hosted::<K, SHARED>(self.b, p, self.st, self.out, &mut self.k) {
            Some(q) => {
                *self.st = State::default();
                self.enter(C_MAIN, q + 1);
                q + 1
            }
            None => self.b.len(),
        }
    }

    /// `main` from `p`: to the line's end, or where a directive's or a block's values start.
    #[inline(always)]
    fn main(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            p = match MAIN[usize::from(b[p])] {
                M_SPACE => p + 1,
                M_WORD => {
                    let e = self.word(p);
                    if self.ctx != C_MAIN {
                        return e;
                    }
                    e
                }
                M_HIGH => {
                    if high_word(b, p) > 0 {
                        word_end(b, p)
                    } else {
                        p + crate::prim::char_len(b[p])
                    }
                }
                M_HASH => {
                    self.put(p, n, COMMENT);
                    n
                }
                _ => p + 1,
            };
        }
        p
    }

    /// A word at `p`, after a byte that isn't a word's: where the scan goes on.
    #[inline(always)]
    fn word(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p + 1;
        while e < n && crate::prim::WORD[usize::from(b[e])] {
            e += 1;
        }
        if e < n && b[e] >= 0x80 && high_word(b, e) > 0 {
            // a word past ASCII: no pattern ends inside it
            return word_end(b, e);
        }
        let i = WORDS.value(&b[p..e], &INFO);
        match i {
            I_NONE => self.other(p, e),
            I_CONTEXT => {
                if self.block_at(e) {
                    self.put(p, e, CONTEXT);
                    e
                } else {
                    self.other(p, e)
                }
            }
            I_TYPES => {
                if self.block_at(e) {
                    self.put(p, e, CONTEXT);
                    e
                } else if b.get(e) == Some(&b' ') && word_at(b, e + 1) {
                    // the http list's `types\ ` (its `\b` after the blank)
                    self.directive(p, e + 1, S_HTTP)
                } else {
                    self.other(p, e)
                }
            }
            I_SERVER => {
                if self.block_at(e) {
                    self.put(p, e, CONTEXT);
                    e
                } else {
                    self.directive(p, e, S_UPSTREAM)
                }
            }
            I_LOCATION | I_UPSTREAM => self.header(p, e, i),
            I_IF => {
                // `\b(if) +\(`
                let q = spaces(b, e);
                if q > e && q < n && b[q] == b'(' {
                    self.put(p, e, CONTROL);
                    self.enter(C_IF, q + 1);
                    q + 1
                } else {
                    self.other(p, e)
                }
            }
            I_MAP => self.map(p, e),
            I_REWRITE => self.rewrite(p, e),
            S_LIMIT_REQ => {
                self.put(p, e, NAME + u16::from(S_LIMIT_REQ));
                e
            }
            S_STUB_STATUS if e == n || BLANK[usize::from(b[e])] => self.unsupported(p, e),
            s => self.directive(p, e, s),
        }
    }

    /// Whether ` +\{` is at `e`.
    #[inline(always)]
    fn block_at(&self, e: usize) -> bool {
        let b = self.b;
        let q = spaces(b, e);
        q > e && q < b.len() && b[q] == b'{'
    }

    /// `location` or `upstream` (`i`) at `p..e`: `\b(location) +[\^]?~[\*]? +(.*?)\{` (a
    /// regex), else `\b(location|upstream) +(.*?)\{` (a name), else another word.
    #[cold]
    #[inline(never)]
    fn header(&mut self, p: usize, e: usize, i: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let q = spaces(b, e);
        if q == e {
            return self.other(p, e);
        }
        let (kind, name) = if i == I_LOCATION {
            (CONTEXT_LOCATION, LOCATION_NAME)
        } else {
            (CONTEXT, LOCATION_NAME)
        };
        if i == I_LOCATION {
            let mut r = q;
            if b.get(r) == Some(&b'^') {
                r += 1;
            }
            if b.get(r) == Some(&b'~') {
                r += 1;
                if b.get(r) == Some(&b'*') {
                    r += 1;
                }
                let s = spaces(b, r);
                let brace = self.next_brace(s);
                if s > r && brace < n {
                    self.put(p, e, kind);
                    self.put(s, brace, REGEXP);
                    return brace + 1;
                }
            }
        }
        let brace = self.next_brace(q);
        if brace == n {
            return self.other(p, e);
        }
        self.put(p, e, kind);
        self.put(q, brace, name);
        brace + 1
    }

    /// `map` at `p..e`: `\b(map) +(\$[A-Za-z0-9\_]+) +(\$[A-Za-z0-9\_]+) *\{`, else another word.
    #[cold]
    #[inline(never)]
    fn map(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let v1 = spaces(b, e);
        let e1 = dollar_word(b, v1);
        let v2 = spaces(b, e1);
        let e2 = dollar_word(b, v2);
        let q = spaces(b, e2);
        if v1 == e || e1 == v1 || v2 == e1 || e2 == v2 || q == n || b[q] != b'{' {
            return self.other(p, e);
        }
        self.put(p, e, CONTEXT);
        self.put(v1, e1, VARIABLE);
        self.put(v2, e2, VARIABLE);
        self.enter(C_MAP, q + 1);
        q + 1
    }

    /// `rewrite` at `p..e`: `\b(rewrite)\b *(".+?(?<!\\)"|'.+?(?<!\\)'|((.+?)(?<!\\)\s))`, the
    /// pattern a regex, then the directive's values; else another word.
    #[cold]
    #[inline(never)]
    fn rewrite(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let q = spaces(b, e);
        let quoted = |c: u8| {
            let mut i = q + 2;
            while i < n {
                if b[i] == c && b[i - 1] != b'\\' {
                    return Some(i + 1);
                }
                i += 1;
            }
            None
        };
        let end = match b.get(q) {
            Some(&c @ (b'"' | b'\'')) => quoted(c),
            _ => None,
        }
        .or_else(|| {
            // the shortest run to a blank (the line's `\n` at its end) after no `\`
            let mut i = q + 1;
            while i <= n {
                if (i == n || BLANK[usize::from(b[i])]) && b[i - 1] != b'\\' {
                    return Some((i + 1).min(n));
                }
                i += 1;
            }
            None
        });
        let Some(end) = end.filter(|_| q < n) else {
            return self.other(p, e);
        };
        self.put(p, e, NAME + u16::from(S_REWRITE));
        self.enter(C_DIRECTIVE, e);
        self.tok(q, end, REGEXP);
        end
    }

    /// A directive `b[p..e]` of scope `s`: its values follow.
    #[inline(always)]
    fn directive(&mut self, p: usize, e: usize, s: u8) -> usize {
        self.put(p, e, NAME + u16::from(s));
        self.enter(C_DIRECTIVE, e);
        e
    }

    /// A word no list has: a Lua block's directive, an unknown directive (a word before a
    /// blank), or plain.
    #[inline(always)]
    fn other(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        if e - p > 13 && b[p..e].ends_with(b"_by_lua_block") {
            self.put(p, e, UNSUPPORTED);
            self.enter(C_LUA_HEAD, e);
            return e;
        }
        if e == b.len() || BLANK[usize::from(b[e])] {
            return self.unsupported(p, e);
        }
        if b[e] == b'/' {
            return self.mime(p, e);
        }
        e
    }

    /// An unknown directive, `\b([a-zA-Z0-9\_]+)\s+` at `p` (its name to `e`): its values
    /// from past the blanks, which it takes.
    #[inline(always)]
    fn unsupported(&mut self, p: usize, e: usize) -> usize {
        self.put(p, e, UNSUPPORTED);
        self.enter(C_UNSUPPORTED, e);
        let q = crate::line_format::blanks(self.b, e);
        self.fill(q);
        q
    }

    /// A word `b[p..e]` before a `/`: a MIME type, `\b([a-z]+\/[a-z0-9\-\.\+]+)\b`, a
    /// directive; else plain.
    #[cold]
    #[inline(never)]
    fn mime(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if !b[p..e].iter().all(u8::is_ascii_lowercase) {
            return e;
        }
        let mut r = e + 1;
        while r < n && matches!(b[r], b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'+') {
            r += 1;
        }
        // back to a `\b`
        let mut t = r;
        while t > e + 1 {
            if crate::prim::WORD[usize::from(b[t - 1])] != word_at(b, t) {
                return self.directive(p, t, S_HTTP);
            }
            t -= 1;
        }
        e
    }

    /// A value list from `p`: to the line's end, or where it ends.
    #[inline(always)]
    fn values(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        while p < n {
            p = match VALUE[usize::from(b[p])] {
                V_BLANK => self.blank(p),
                V_SEMI => match self.ctx {
                    C_DIRECTIVE | C_UNSUPPORTED | C_LUA_HEAD => {
                        self.fill(p + 1);
                        self.enter(C_MAIN, p + 1);
                        return p + 1;
                    }
                    C_MAP => {
                        self.tok(p, p + 1, MAP_SEPARATOR);
                        p + 1
                    }
                    _ => p + 1,
                },
                V_RPAREN if self.ctx == C_IF => {
                    self.enter(C_MAIN, p + 1);
                    return p + 1;
                }
                V_RBRACE if self.ctx == C_MAP => {
                    self.enter(C_MAIN, p + 1);
                    return p + 1;
                }
                V_LBRACE if self.ctx == C_LUA_HEAD => {
                    self.fill(p + 1);
                    *self.st = State::default();
                    self.enter(C_LUA, p + 1);
                    return p + 1;
                }
                V_HASH if self.ctx == C_MAP => {
                    self.tok(p, n, COMMENT);
                    n
                }
                V_DOLLAR => self.variable(p),
                V_QUOTE => {
                    let q = b[p];
                    self.string = if q == b'"' { S_DOUBLE } else { S_SINGLE };
                    let base = self.string_base();
                    self.tok(p, p + 1, base);
                    self.string_rest(p + 1)
                }
                V_TILDE => {
                    let l = if b.get(p + 1) == Some(&b'*') { 2 } else { 1 };
                    self.tok(p, p + l, REGEXP);
                    p + l
                }
                V_BACKSLASH => {
                    // `\\.*\ `: to the line's last space
                    let l = self.last_space();
                    if l > p {
                        self.tok(p, l + 1, REGEXP);
                        l + 1
                    } else if matches!(b.get(p + 1), Some(b'"' | b'\'')) {
                        // an escaped quote, as nginx reads it (a fix: bat opens a string)
                        p + 2
                    } else {
                        p + 1
                    }
                }
                V_CARET => {
                    // `\^.*?\$`
                    let i = self.next_dollar(p + 1);
                    if i < n {
                        self.tok(p, i + 1, REGEXP);
                        i + 1
                    } else {
                        p + 1
                    }
                }
                V_BANG if b.get(p + 1) == Some(&b'~') => {
                    let l = if b.get(p + 2) == Some(&b'*') { 3 } else { 2 };
                    self.tok(p, p + l, REGEXP);
                    p + l
                }
                _ => p + 1,
            };
        }
        p
    }

    /// `\$[A-Za-z0-9_]+\b` at `p`: where the scan goes on.
    #[inline(always)]
    fn variable(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        while e < b.len() && crate::prim::WORD[usize::from(b[e])] {
            e += 1;
        }
        if e > p + 1 && !(e < b.len() && b[e] >= 0x80 && word_at(b, e)) {
            self.tok(p, e, VARIABLE);
            e
        } else {
            p + 1
        }
    }

    /// A blank at `p`: bat's patterns that start with one (`[\t ]…(?=[\t ;])`), an address, a
    /// number, a constant; where the scan goes on.
    #[inline(always)]
    fn blank(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let s = p + 1;
        let mut t = s;
        while t < n && !VALUE_END[usize::from(b[t])] {
            t += 1;
        }
        if t == s || t == n {
            return s;
        }
        let w = &b[s..t];
        if let Some(a) = address(w) {
            self.tok(s, s + a, IP);
            if a < w.len() {
                self.tok(s + a, t, CIDR);
            }
            return t;
        }
        if number(w) {
            self.tok(s, t, NUMBER);
            return t;
        }
        if constant(w) {
            self.tok(p, t, CONSTANT);
            return t;
        }
        s
    }

    /// Where the line's last space is (`\\.*\ `'s end), 0 for none: found once a line.
    #[inline(always)]
    fn last_space(&mut self) -> usize {
        if self.space == usize::MAX {
            self.space = self.b.iter().rposition(|&c| c == b' ').unwrap_or(0);
        }
        self.space
    }

    /// Where the first `$` at or past `p` is, or the line's end; each search starts past the
    /// last one's answer, so a line's searches are linear together.
    #[inline(always)]
    fn next_dollar(&mut self, p: usize) -> usize {
        if self.dollar < p {
            self.dollar = crate::prim::find_any(self.b, p, *b"$");
        }
        self.dollar
    }

    /// As [`Scan::next_dollar`], for a header's `{`.
    #[inline(always)]
    fn next_brace(&mut self, p: usize) -> usize {
        if self.brace < p {
            self.brace = crate::prim::find_any(self.b, p, *b"{");
        }
        self.brace
    }

    /// The open string's text kind.
    #[inline(always)]
    fn string_base(&self) -> u16 {
        if self.string == S_DOUBLE {
            DOUBLE
        } else {
            SINGLE
        }
    }

    /// An open string from `p`: its escapes and variables, to its quote or the line's end. Out
    /// of line: one copy for a line's start and a value's quote.
    #[inline(never)]
    fn string_rest(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let base = self.string_base();
        let q = if self.string == S_DOUBLE { b'"' } else { b'\'' };
        loop {
            let i = crate::prim::find_any(b, p, [q, b'\\', b'$']);
            if i >= n {
                self.tok(p, n, base);
                return n;
            }
            let c = b[i];
            if c == q {
                self.tok(p, i + 1, base);
                self.string = S_NONE;
                return i + 1;
            }
            if c == b'\\' {
                if b.get(i + 1) == Some(&q) {
                    self.tok(p, i, base);
                    self.tok(i, i + 2, base + ESCAPE);
                    p = i + 2;
                } else {
                    self.tok(p, i + 1, base);
                    p = i + 1;
                }
                continue;
            }
            let mut e = i + 1;
            while e < n && crate::prim::WORD[usize::from(b[e])] {
                e += 1;
            }
            if e > i + 1 && !(e < n && b[e] >= 0x80 && word_at(b, e)) {
                self.tok(p, i, base);
                self.tok(i, e, base + STRING_VARIABLE);
            } else {
                self.tok(p, e, base);
            }
            p = e;
        }
    }
}

/// `([0-9]{1,3}(\.[0-9]{1,3}){3}|[0-9a-f:]+)(\/[0-9]{2})?` as all of `w`: where the address
/// ends (the rest its CIDR suffix).
#[inline(always)]
fn address(w: &[u8]) -> Option<usize> {
    let a = match w {
        [x @ .., b'/', d1, d2] if d1.is_ascii_digit() && d2.is_ascii_digit() => x.len(),
        _ => w.len(),
    };
    let x = &w[..a];
    if x.is_empty() {
        return None;
    }
    if x.iter()
        .all(|&c| matches!(c, b'0'..=b'9' | b'a'..=b'f' | b':'))
        || v4(x)
    {
        Some(a)
    } else {
        None
    }
}

/// `[0-9]{1,3}(\.[0-9]{1,3}){3}` as all of `x`.
#[inline(always)]
fn v4(x: &[u8]) -> bool {
    let mut parts = 0;
    for part in x.split(|&c| c == b'.') {
        parts += 1;
        if part.is_empty() || part.len() > 3 || !part.iter().all(u8::is_ascii_digit) {
            return false;
        }
    }
    parts == 4
}

/// `=?[0-9][0-9\.]*[bBkKmMgGtTsShHdD]?` as all of `w`.
#[inline(always)]
fn number(w: &[u8]) -> bool {
    let w = w.strip_prefix(b"=").unwrap_or(w);
    let w = match w {
        [x @ .., s] if b"bBkKmMgGtTsShHdD".contains(s) => x,
        _ => w,
    };
    !w.is_empty() && w[0].is_ascii_digit() && w.iter().all(|&c| c.is_ascii_digit() || c == b'.')
}

/// bat's constants after a blank, as all of `w`.
#[inline(always)]
fn constant(w: &[u8]) -> bool {
    matches!(
        w,
        b"on"
            | b"off"
            | b"true"
            | b"false"
            | b"kqueue"
            | b"rtsig"
            | b"epoll"
            | b"/dev/poll"
            | b"select"
            | b"poll"
            | b"eventport"
            | b"max"
            | b"all"
            | b"default_server"
            | b"default"
            | b"main"
            | b"crit"
            | b"error"
            | b"debug"
            | b"warn"
            | b"notice"
            | b"last"
    )
}

#[cfg(test)]
mod tests {
    use crate::{Lang, State, lex};

    /// Each line's tokens as `text·leaf`, the leaf a kind's last scope without its suffix, plain
    /// text left out.
    fn toks(text: &str) -> Vec<String> {
        let mut state = State::default();
        let mut spans = Vec::new();
        text.lines()
            .map(|l| {
                lex(Lang::Nginx, l, &mut state, &mut spans);
                spans
                    .iter()
                    .map(|s| {
                        let scopes = Lang::Nginx.scopes(s.kind);
                        let leaf = scopes.last().map_or("?", |s| {
                            [".nginx", ".lua"]
                                .iter()
                                .find_map(|x| s.strip_suffix(x))
                                .unwrap_or(s)
                        });
                        format!("{}·{leaf}", &l[s.start as usize..s.end as usize])
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn directives_and_values() {
        let t = toks(concat!(
            "user  nginx;\n",
            "worker_connections  1024;\n",
            "listen 80 default_server;\n",
            "allow 192.168.1.0/24;\n",
            "client_max_body_size 10m;\n",
            "set $a \"x\\\"y $b\";\n",
            "foo_bar 80 on;\n",
            "stub_status;\n",
            "limit_req zone=one burst=5;\n",
        ));
        assert_eq!(
            t[0],
            "user·keyword.directive   nginx;·punctuation.definition.variable"
        );
        assert_eq!(
            t[1],
            "worker_connections·keyword.directive.module.events   ·punctuation.definition.variable \
             1024·string.ipaddress ;·punctuation.definition.variable"
        );
        assert_eq!(
            t[2],
            "listen·keyword.directive.module.http  ·punctuation.definition.variable 80·string.ipaddress  \
             default_server·constant.language ;·punctuation.definition.variable"
        );
        assert_eq!(
            t[3],
            "allow·keyword.directive.module.http.access  ·punctuation.definition.variable \
             192.168.1.0·string.ipaddress /24·constant.numeric.cidr ;·punctuation.definition.variable"
        );
        assert_eq!(
            t[4],
            "client_max_body_size·keyword.directive.module.http  ·punctuation.definition.variable \
             10m·constant.numeric ;·punctuation.definition.variable"
        );
        assert_eq!(
            t[5],
            "set·keyword.directive.module.http.rewrite  ·punctuation.definition.variable $a·variable.other  \
             ·punctuation.definition.variable \"x·string.quoted.double \\\"·constant.character.escape \
             y ·string.quoted.double $b·variable.other \"·string.quoted.double ;·punctuation.definition.variable"
        );
        // the unknown directive's `\s+` takes the blank a value would start with
        assert_eq!(
            t[6],
            "foo_bar·keyword.directive.module.unsupported  80·punctuation.definition.variable  \
             on·constant.language ;·punctuation.definition.variable"
        );
        assert_eq!(
            t[7],
            "stub_status·keyword.directive.module.http.stub_status ;·punctuation.definition.variable"
        );
        assert_eq!(
            t[8],
            "limit_req·keyword.directive.module.http.limit_req one·keyword.directive.module.unsupported  \
             burst=5;·punctuation.definition.variable"
        );
    }

    #[test]
    fn blocks_and_headers() {
        let t = toks(concat!(
            "http {\n",
            "location ~* \\.(jpg|png)$ {\n",
            "location = /50x.html {\n",
            "location ^~ /static/ { alias /var/www/; }\n",
            "upstream backend {\n",
            "if ($http_user_agent ~ MSIE) {\n",
            "rewrite ^(.*)$ /msie/$1 break;\n",
            "map $http_upgrade $connection_upgrade {\n",
            "    default upgrade;\n",
            "    ''      close; # c\n",
            "}\n",
            "types {\n",
            "    text/html  html htm;\n",
            "}\n",
        ));
        assert_eq!(t[0], "http·storage.type.context");
        assert_eq!(
            t[1],
            "location·storage.type.context.location \\.(jpg|png)$ ·string.regexp"
        );
        assert_eq!(
            t[2],
            "location·storage.type.context.location = /50x.html ·entity.name.context.location"
        );
        assert_eq!(
            t[3],
            "location·storage.type.context.location /static/ ·string.regexp \
             alias·keyword.directive.module.http  /var/www/;·punctuation.definition.variable"
        );
        assert_eq!(
            t[4],
            "upstream·storage.type.context backend ·entity.name.context.location"
        );
        assert_eq!(
            t[5],
            "if·keyword.control $http_user_agent·variable.other ~·string.regexp"
        );
        assert_eq!(
            t[6],
            "rewrite·keyword.directive.module.http.rewrite  ·punctuation.definition.variable \
             ^(.*)$ ·string.regexp /msie/·punctuation.definition.variable $1·variable.other  \
             break;·punctuation.definition.variable"
        );
        assert_eq!(
            t[7],
            "map·storage.type.context $http_upgrade·variable.other $connection_upgrade·variable.other"
        );
        assert_eq!(
            t[8],
            " default·constant.language ;·punctuation.definition.map"
        );
        assert_eq!(
            t[9],
            "''·string.quoted.single ;·punctuation.definition.map # c·comment.line.number-sign"
        );
        assert_eq!(t[11], "types·storage.type.context");
        assert_eq!(
            t[12],
            "text/html·keyword.directive.module.http   html htm;·punctuation.definition.variable"
        );
    }

    #[test]
    fn lines_carry_directives_and_strings() {
        let t = toks(concat!(
            "log_format main '$remote_addr - '\n",
            "    '\"$request\" $status';\n",
            "error_log off\n",
            "    ;\n",
            "# a comment\n",
        ));
        assert_eq!(
            t[1],
            "    ·punctuation.definition.variable '\"·string.quoted.single \
             $request·variable.other \" ·string.quoted.single $status·variable.other \
             '·string.quoted.single ;·punctuation.definition.variable"
        );
        assert_eq!(t[3], "    ;·punctuation.definition.variable");
        assert_eq!(t[4], "# a comment·comment.line.number-sign");
    }

    #[test]
    fn lua_blocks_end_where_nginx_ends_them() {
        let t = toks(concat!(
            "location /lua {\n",
            "    content_by_lua_block {\n",
            "        local t = { a = \"}\" } -- }\n",
            "        local s = [[ }\n",
            "        ]]\n",
            "    }\n",
            "    access_by_lua_block { ngx.exit(403) } set $x 1;\n",
            "    set_by_lua_block $res { return 32 }\n",
            "}\n",
        ));
        assert_eq!(
            t[1],
            "content_by_lua_block·keyword.directive.module.unsupported  {·punctuation.definition.variable"
        );
        // a `}` in a string, a comment or a long bracket, or a table's, isn't the block's
        assert!(
            t[2].ends_with(
                "\"·punctuation.definition.string.begin }·string.quoted.double \
                 \"·punctuation.definition.string.end }·punctuation.section.block.end \
                 --·punctuation.definition.comment  }·comment.line"
            ),
            "{}",
            t[2]
        );
        assert!(t[3].ends_with(" }·string.quoted.multiline"), "{}", t[3]);
        assert_eq!(t[5], "");
        assert!(
            t[6].ends_with(
                "set·keyword.directive.module.http.rewrite  ·punctuation.definition.variable \
                 $x·variable.other  ·punctuation.definition.variable 1·string.ipaddress \
                 ;·punctuation.definition.variable"
            ),
            "{}",
            t[6]
        );
        assert!(t[7].contains("$res·variable.other"), "{}", t[7]);
        assert!(
            t[7].contains("return·keyword.control.return 32·constant.numeric"),
            "{}",
            t[7]
        );
    }

    #[test]
    fn escaped_quotes_and_words_past_ascii() {
        let t = toks(concat!(
            "set $a \\'$b\\';\n",
            "set $c 1;\n",
            "\u{feff}user www;\n",
        ));
        assert!(t[1].starts_with("set·keyword"), "{}", t[1]);
        assert!(t[2].starts_with("user·keyword.directive "), "{}", t[2]);
    }

    #[test]
    fn long_lines_lex_in_linear_time() {
        for unit in [
            "\\",
            "^",
            "location ",
            "location ~ ",
            "rewrite ",
            "a/",
            " 1",
            "'\\'",
            "map ",
            "$",
            "x_by_lua_block { { ",
            "if ( ",
        ] {
            for head in ["", "set $x ", "location / {"] {
                let line = format!("{head}{}", unit.repeat(100_000));
                let t = std::time::Instant::now();
                toks(&line);
                assert!(t.elapsed().as_secs() < 2, "{head:?} {unit:?}");
            }
        }
    }
}
