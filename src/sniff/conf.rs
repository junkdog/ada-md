//! Configuration files by their lines' shape (FEAT-020): known_hosts, pkg-config, fstab, a
//! crontab, `.gitattributes`, `.mailmap`, ssh's and sshd's configs, `.env`, and the key-value
//! family (TOML, INI, Java properties) told apart by their values. Each wants most of its lines
//! in its shape, comments and blank lines aside; `.gitignore`'s globs, the loosest, are tried
//! last of all, by the caller.

use std::sync::LazyLock;

use ada_parse::Lang;
use regex::Regex;

fn re(s: &str) -> Regex {
    Regex::new(s).unwrap()
}

/// Lines with content that aren't `#` comments.
fn content<'a>(lines: &'a [&'a str]) -> impl Iterator<Item = &'a str> + Clone {
    lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
}

/// At least one content line, and nine in ten in the shape.
fn most(lines: &[&str], shaped: impl Fn(&str) -> bool) -> bool {
    let (yes, total) = content(lines).fold((0, 0), |(y, t), l| (y + usize::from(shaped(l)), t + 1));
    yes > 0 && yes * 10 >= total * 9
}

/// A configuration format whose lines show it surely.
pub(super) fn sure(lines: &[&str]) -> Option<Lang> {
    static KNOWN_HOST: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r"^(@(cert-authority|revoked) )?\S+\s+(ssh-(rsa|ed25519|dss)|ecdsa-sha2-nistp\d+|sk-\S+@openssh\.com|ssh-\S+-cert-v01@openssh\.com)\s+AAAA",
        )
    });
    static PKG_FIELD: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r"^(Name|Description|Version|URL|Requires|Requires\.private|Conflicts|Libs|Libs\.private|Cflags):",
        )
    });
    static PKG_VAR: LazyLock<Regex> = LazyLock::new(|| re(r"^[\w.]+\s*=.*$"));
    static FSTAB: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r"^(UUID=|LABEL=|PARTUUID=|PARTLABEL=|/dev/|tmpfs|proc|sysfs|none|devpts|efivarfs|\S+:/|//)\S*\s+(/\S*|none|swap)\s+[\w.,]+\s+\S+(\s+\d+(\s+\d+)?)?$",
        )
    });
    static ATTRS: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r"^(\S+|\[attr\]\S+)(\s+[-!]?(text|binary|eol|diff|merge|filter|crlf|whitespace|working-tree-encoding|linguist-[\w-]+|export-ignore|export-subst|lockable|ident|delta|encoding)(=\S+)?)+$",
        )
    });
    static MAILMAP: LazyLock<Regex> =
        LazyLock::new(|| re(r"^([^<#]*\S\s+)?<[^<>\s]*>(\s+[^<#]*\S)?(\s+<[^<>\s]*>)?\s*(#.*)?$"));
    let content_count = content(lines).count();
    if content_count == 0 {
        return crontab_template(lines).then_some(Lang::Crontab);
    }
    if most(lines, |l| KNOWN_HOST.is_match(l)) {
        return Some(Lang::KnownHosts);
    }
    // a field's line, not a struct literal's (`Name: "x",`); and a `Libs:` or `Cflags:` among them
    let field = |l: &str| PKG_FIELD.is_match(l) && !l.ends_with([',', '{', '(']);
    let fields = content(lines).filter(|l| field(l)).count();
    let flags = content(lines).any(|l| l.starts_with("Libs:") || l.starts_with("Cflags:"));
    let shaped = most(lines, |l| field(l) || PKG_VAR.is_match(l));
    if flags && fields >= 2 || fields >= 3 && shaped {
        return Some(Lang::PkgConfig);
    }
    if most(lines, |l| FSTAB.is_match(l)) {
        return Some(Lang::Fstab);
    }
    if is_crontab(lines) {
        return Some(Lang::Crontab);
    }
    if most(lines, |l| ATTRS.is_match(l)) {
        return Some(Lang::GitAttributes);
    }
    if content(lines).any(|l| l.contains('@')) && most(lines, |l| MAILMAP.is_match(l)) {
        return Some(Lang::GitMailmap);
    }
    ssh(lines)
}

/// A crontab: schedules and variables, or only the commented header `crontab -e` writes.
fn is_crontab(lines: &[&str]) -> bool {
    static SCHEDULE: LazyLock<Regex> = LazyLock::new(|| {
        let field = r"([\d*/,-]+|[A-Za-z]{3}([,-][A-Za-z]{3})*)";
        re(&format!(
            r"^(@(reboot|yearly|annually|monthly|weekly|daily|midnight|hourly)|{field}(\s+{field}){{4}})\s+\S"
        ))
    });
    static VAR: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Za-z_]\w*\s*=\s*\S"));
    content(lines).any(|l| SCHEDULE.is_match(l))
        && most(lines, |l| SCHEDULE.is_match(l) || VAR.is_match(l))
        || crontab_template(lines)
}

/// The header of a crontab's comments: `# m h  dom mon dow   command`.
fn crontab_template(lines: &[&str]) -> bool {
    static HEADER: LazyLock<Regex> = LazyLock::new(|| {
        re(r"^#\s*(m|min(ute)?)\s+(h|hour)\s+(dom|day)\s+(mon|month)\s+(dow|weekday)")
    });
    lines.iter().any(|l| HEADER.is_match(l))
}

/// ssh's client config or sshd's: lines of a keyword and its value, commented ones counted too
/// (a stock config is mostly those), the keywords only one of them takes deciding.
fn ssh(lines: &[&str]) -> Option<Lang> {
    const CLIENT: &[&str] = &[
        "host",
        "hostname",
        "user",
        "identityfile",
        "identitiesonly",
        "proxyjump",
        "proxycommand",
        "forwardagent",
        "forwardx11",
        "forwardx11trusted",
        "stricthostkeychecking",
        "userknownhostsfile",
        "globalknownhostsfile",
        "serveraliveinterval",
        "serveralivecountmax",
        "sendenv",
        "hashknownhosts",
        "addkeystoagent",
        "controlmaster",
        "controlpath",
        "controlpersist",
        "localforward",
        "remoteforward",
        "dynamicforward",
        "batchmode",
        "checkhostip",
        "connecttimeout",
        "preferredauthentications",
        "usekeychain",
        "requesttty",
        "verifyhostkeydns",
        "canonicalizehostname",
        "identityagent",
        "hostkeyalias",
        "escapechar",
        "tunnel",
        "hostkeyalgorithms",
        "visualhostkey",
        "connectionattempts",
        "updatehostkeys",
        "gssapidelegatecredentials",
    ];
    const SERVER: &[&str] = &[
        "permitrootlogin",
        "challengeresponseauthentication",
        "kbdinteractiveauthentication",
        "usepam",
        "subsystem",
        "authorizedkeysfile",
        "acceptenv",
        "x11forwarding",
        "printmotd",
        "printlastlog",
        "hostkey",
        "listenaddress",
        "permitemptypasswords",
        "maxauthtries",
        "maxsessions",
        "maxstartups",
        "clientaliveinterval",
        "clientalivecountmax",
        "allowusers",
        "allowgroups",
        "denyusers",
        "denygroups",
        "logingracetime",
        "strictmodes",
        "syslogfacility",
        "usedns",
        "permittunnel",
        "permituserenvironment",
        "allowtcpforwarding",
        "allowagentforwarding",
        "gatewayports",
        "banner",
        "chrootdirectory",
        "forcecommand",
        "authorizedkeyscommand",
        "authorizedkeyscommanduser",
        "trustedusercakeys",
        "pidfile",
        "x11displayoffset",
        "x11uselocalhost",
        "authorizedprincipalsfile",
        "permitopen",
        "permitlisten",
        "hostbasedusesnamefrompacketonly",
    ];
    const BOTH: &[&str] = &[
        "port",
        "ciphers",
        "macs",
        "kexalgorithms",
        "match",
        "include",
        "loglevel",
        "gssapiauthentication",
        "gssapicleanupcredentials",
        "pubkeyauthentication",
        "passwordauthentication",
        "hostbasedauthentication",
        "compression",
        "tcpkeepalive",
        "addressfamily",
        "pubkeyacceptedkeytypes",
        "pubkeyacceptedalgorithms",
        "rekeylimit",
        "securitykeyprovider",
        "casignaturealgorithms",
        "fingerprinthash",
        "ipqos",
        "streamlocalbindunlink",
        "setenv",
    ];
    let keyword = |l: &str| {
        let l = l.trim_start_matches('#').trim_start();
        let word = l.split([' ', '\t', '=']).next().unwrap_or_default();
        let rest = l[word.len()..].trim_start_matches([' ', '\t', '=']);
        if word.is_empty() || rest.is_empty() || !word.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        let w = word.to_ascii_lowercase();
        let w = w.as_str();
        if CLIENT.contains(&w) {
            Some(Some(Lang::SshConfig))
        } else if SERVER.contains(&w) {
            Some(Some(Lang::SshdConfig))
        } else if BOTH.contains(&w) {
            Some(None)
        } else {
            None
        }
    };
    // every content line a keyword's, and at least two keyword lines with commented ones
    if !most(lines, |l| keyword(l).is_some()) {
        let commented = lines
            .iter()
            .filter(|l| l.trim_start().starts_with('#') && keyword(l).is_some())
            .count();
        if content(lines).next().is_some() || commented < 3 {
            return None;
        }
    }
    let (mut client, mut server) = (0, 0);
    for l in lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        match keyword(l) {
            Some(Some(Lang::SshConfig)) => client += 1,
            Some(Some(Lang::SshdConfig)) => server += 1,
            _ => {}
        }
    }
    if client + server < 2 {
        return None;
    }
    Some(if server > client {
        Lang::SshdConfig
    } else {
        Lang::SshConfig
    })
}

/// `.env`: `NAME=value` lines, no spaces around `=`, names in upper case.
pub(super) fn is_dotenv(lines: &[&str]) -> bool {
    static ASSIGN: LazyLock<Regex> =
        LazyLock::new(|| re(r"^(export\s+)?[A-Za-z_][A-Za-z0-9_]*=(\S|$)"));
    let upper = |l: &str| {
        let name = l.trim_start_matches("export").trim_start();
        let name = name.split('=').next().unwrap_or_default();
        !name.bytes().any(|b| b.is_ascii_lowercase())
    };
    most(lines, |l| ASSIGN.is_match(l))
        && content(lines).filter(|l| upper(l)).count() * 2 >= content(lines).count()
}

/// The key-value family, once [`super`]'s TOML shape holds: TOML where the values are TOML's,
/// INI where a `[section]` holds bare values, Java properties where none does.
pub(super) fn key_values(lines: &[&str]) -> Lang {
    static SECTION: LazyLock<Regex> = LazyLock::new(|| re(r"^\[[^\]]+\]\s*([#;].*)?$"));
    static PAIR: LazyLock<Regex> =
        LazyLock::new(|| re(r"^([^=:\s]+(\s*\.\s*[^=:\s]+)*)\s*[=:]\s*(.*)$"));
    static TOML_VALUE: LazyLock<Regex> = LazyLock::new(|| {
        re(r#"^("|'|\[|\{|[+-]?(\d|inf\b|nan\b)|true\b|false\b|\d{4}-\d{2}-\d{2})"#)
    });
    let semicolon = lines.iter().any(|l| l.trim_start().starts_with(';'));
    let sections = content(lines).filter(|l| SECTION.is_match(l)).count();
    let (mut toml, mut bare) = (0, 0);
    for l in content(lines) {
        if let Some(c) = PAIR.captures(l) {
            let value = c.get(3).map_or("", |m| m.as_str());
            if TOML_VALUE.is_match(value) {
                toml += 1;
            } else {
                bare += 1;
            }
        }
    }
    if !semicolon && toml >= bare * 3 {
        Lang::Toml
    } else if sections > 0 || semicolon {
        Lang::Ini
    } else {
        Lang::Properties
    }
}

/// Java properties the TOML shape misses: `key=value` lines whose keys hold what a TOML key
/// can't (`/`, `@`, `\:`), at least two of them.
pub(super) fn is_properties(lines: &[&str]) -> bool {
    static PROPERTY: LazyLock<Regex> = LazyLock::new(|| re(r"^[^\s=#!]([^\s=]|\\ )*\s*=(\S|\s*$)"));
    content(lines).count() >= 2 && most(lines, |l| PROPERTY.is_match(l) || l.starts_with('!'))
}

/// `.gitignore`: globs and paths, one a line, no spaces, some with a glob's or a path's marks.
pub(super) fn is_gitignore(lines: &[&str]) -> bool {
    let glob = |l: &str| !l.contains(char::is_whitespace) || l.contains("\\ ");
    let marked = |l: &str| {
        l.contains(['*', '?', '['])
            || l.starts_with(['/', '!'])
            || l.ends_with('/')
            || l.starts_with('.')
    };
    most(lines, glob) && content(lines).filter(|l| marked(l)).count() * 3 >= content(lines).count()
}
