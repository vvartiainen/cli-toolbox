use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, exec, select};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub name: String,
}

pub fn select_and_connect_host(
    home: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let config_path = home.join(".ssh").join("config");
    let hosts = load_hosts(&config_path)?;
    if hosts.is_empty() {
        return Err(Error::msg(format!(
            "no ssh hosts found in \"{}\"",
            config_path.display()
        )));
    }

    let Some(selected) = select_host(&hosts)? else {
        return Ok(());
    };
    connect(selected, stdout, stderr)
}

pub fn load_hosts(config_path: &Path) -> Result<Vec<Host>, Error> {
    let mut loader = HostLoader {
        seen: HashSet::new(),
        hosts: BTreeMap::new(),
    };
    loader.load_file(config_path)?;
    Ok(loader.hosts.into_values().collect())
}

struct HostLoader {
    seen: HashSet<PathBuf>,
    hosts: BTreeMap<String, Host>,
}

impl HostLoader {
    fn load_file(&mut self, path: &Path) -> Result<(), Error> {
        let clean = clean_path(path);
        if !self.seen.insert(clean.clone()) {
            return Ok(());
        }

        let file = match File::open(&clean) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(Error::msg(format!("open {}: {err}", clean.display()))),
        };

        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line =
                line.map_err(|err| Error::msg(format!("read {}: {err}", clean.display())))?;
            let Some((key, args)) = parse_directive(&line) else {
                continue;
            };
            match key.as_str() {
                "host" => {
                    for pattern in args {
                        if !is_concrete_host_pattern(&pattern) {
                            continue;
                        }
                        self.hosts
                            .entry(pattern.clone())
                            .or_insert(Host { name: pattern });
                    }
                }
                "include" => {
                    for include_path in args {
                        self.load_includes(
                            clean.parent().unwrap_or(Path::new(".")),
                            &include_path,
                        )?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn load_includes(&mut self, base_dir: &Path, pattern: &str) -> Result<(), Error> {
        let mut pattern = expand_home(pattern);
        if !pattern.is_absolute() {
            pattern = base_dir.join(pattern);
        }
        let matches = glob_paths(&pattern)
            .map_err(|err| Error::msg(format!("glob include {}: {err}", pattern.display())))?;
        for matched in matches {
            self.load_file(&matched)?;
        }
        Ok(())
    }
}

fn parse_directive(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let key_end = line.find([' ', '\t', '=']).unwrap_or(line.len());
    if key_end == 0 {
        return None;
    }
    let key = line[..key_end].to_ascii_lowercase();
    let mut rest = line[key_end..].trim_start();
    if let Some(stripped) = rest.strip_prefix('=') {
        rest = stripped.trim_start();
    }
    let args = split_ssh_fields(rest);
    if args.is_empty() {
        None
    } else {
        Some((key, args))
    }
}

fn split_ssh_fields(input: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let flush = |current: &mut String, fields: &mut Vec<String>| {
        if !current.is_empty() {
            fields.push(std::mem::take(current));
        }
    };

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '#' => {
                flush(&mut current, &mut fields);
                return fields;
            }
            ' ' | '\t' => flush(&mut current, &mut fields),
            _ => current.push(ch),
        }
    }
    flush(&mut current, &mut fields);
    fields
}

fn is_concrete_host_pattern(pattern: &str) -> bool {
    !pattern.is_empty() && !pattern.starts_with('!') && !pattern.contains(['*', '?'])
}

fn expand_home(path: &str) -> PathBuf {
    if path != "~" && !path.starts_with("~/") {
        return PathBuf::from(path);
    }
    let Some(home) = dirs::home_dir() else {
        return PathBuf::from(path);
    };
    if path == "~" {
        home
    } else {
        home.join(&path[2..])
    }
}

fn select_host(hosts: &[Host]) -> Result<Option<&Host>, Error> {
    let lines: Vec<String> = hosts.iter().map(|host| host.name.clone()).collect();
    let Some(index) = select::pick("ssh host> ", &lines, "~100%")? else {
        return Ok(None);
    };
    hosts
        .get(index)
        .ok_or_else(|| Error::msg("selected ssh host was not found"))
        .map(Some)
}

pub(crate) fn connect(
    host: &Host,
    _stdout: &mut dyn Write,
    _stderr: &mut dyn Write,
) -> Result<(), Error> {
    let (cmd_name, args) = kitten_ssh_command(&host.name)?;
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    exec::run_inherited(cmd_name, &arg_refs).map_err(|err| {
        Error::msg(format!(
            "connect to ssh host \"{}\" with kitten ssh: {err}",
            host.name
        ))
    })
}

fn kitten_ssh_command(host: &str) -> Result<(&'static str, Vec<String>), Error> {
    if exec::look_path("kitten").is_some() {
        return Ok(("kitten", vec!["ssh".into(), host.into()]));
    }
    if exec::look_path("kitty").is_some() {
        return Ok(("kitty", vec!["+kitten".into(), "ssh".into(), host.into()]));
    }
    Err(Error::msg("neither kitten nor kitty is available on PATH"))
}

fn glob_paths(pattern: &Path) -> std::io::Result<Vec<PathBuf>> {
    let pattern_str = pattern.to_string_lossy();
    let has_meta = pattern_str.contains(['*', '?', '[']);
    if !has_meta {
        return Ok(if pattern.exists() {
            vec![pattern.to_path_buf()]
        } else {
            Vec::new()
        });
    }

    let mut prefix = PathBuf::new();
    let mut remaining = Vec::new();
    let mut seen_meta = false;
    for component in pattern.components() {
        let text = component.as_os_str().to_string_lossy();
        if !seen_meta && !text.contains(['*', '?', '[']) {
            prefix.push(component);
        } else {
            seen_meta = true;
            remaining.push(text.into_owned());
        }
    }

    let start = if prefix.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        prefix
    };
    let mut matches = Vec::new();
    walk_glob(&start, &remaining, &mut matches)?;
    matches.sort();
    Ok(matches)
}

fn walk_glob(dir: &Path, parts: &[String], out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if parts.is_empty() {
        out.push(dir.to_path_buf());
        return Ok(());
    }
    let (head, tail) = parts.split_first().unwrap();
    if !head.contains(['*', '?', '[']) {
        return walk_glob(&dir.join(head), tail, out);
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if glob_match(head, &name_str) {
            walk_glob(&entry.path(), tail, out)?;
        }
    }
    Ok(())
}

fn glob_match(pattern: &str, name: &str) -> bool {
    glob_match_bytes(pattern.as_bytes(), name.as_bytes())
}

fn glob_match_bytes(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.first() {
        None => name.is_empty(),
        Some(&b'*') => {
            glob_match_bytes(&pattern[1..], name)
                || (!name.is_empty() && glob_match_bytes(pattern, &name[1..]))
        }
        Some(&b'?') => !name.is_empty() && glob_match_bytes(&pattern[1..], &name[1..]),
        Some(&ch) => name.first() == Some(&ch) && glob_match_bytes(&pattern[1..], &name[1..]),
    }
}

fn clean_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{load_hosts, parse_directive};
    use std::fs;
    use std::path::Path;

    #[test]
    fn load_hosts_reads_config_and_includes() {
        let dir = tempfile::tempdir().unwrap();
        let ssh_dir = dir.path().join(".ssh");
        fs::create_dir_all(ssh_dir.join("conf.d")).unwrap();
        let config_path = ssh_dir.join("config");
        let include_path = ssh_dir.join("conf.d").join("work.conf");

        fs::write(
            &config_path,
            r#"
Host dev prod *.ignored !blocked
  User vvarti

Include conf.d/*.conf
"#,
        )
        .unwrap();
        fs::write(
            &include_path,
            r#"
Host bastion
  HostName bastion.example.com

Host "quoted-host"
  HostName quoted.example.com
"#,
        )
        .unwrap();

        let hosts = load_hosts(&config_path).unwrap();
        let names: Vec<_> = hosts.iter().map(|host| host.name.as_str()).collect();
        assert_eq!(names, ["bastion", "dev", "prod", "quoted-host"]);
    }

    #[test]
    fn load_hosts_missing_config_is_empty() {
        let hosts = load_hosts(Path::new("/missing/ssh/config")).unwrap();
        assert!(hosts.is_empty());
    }

    #[test]
    fn parse_directive_supports_equals_and_comments() {
        let (key, args) = parse_directive(r#"Include = "conf.d/*.conf" extra.conf # comment"#)
            .expect("directive");
        assert_eq!(key, "include");
        assert_eq!(args, ["conf.d/*.conf", "extra.conf"]);
    }
}
