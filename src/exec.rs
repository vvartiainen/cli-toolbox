use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::Error;

pub fn look_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var).find_map(|dir| {
        let candidate = dir.join(name);
        is_executable(&candidate).then_some(candidate)
    })
}

pub fn require(name: &str) -> Result<PathBuf, Error> {
    look_path(name).ok_or_else(|| Error::msg(format!("{name} is not available on PATH")))
}

pub fn require_any<'a>(names: &[&'a str]) -> Result<&'a str, Error> {
    for name in names {
        if look_path(name).is_some() {
            return Ok(*name);
        }
    }

    match names {
        [first, second] => Err(Error::msg(format!(
            "neither {first} nor {second} is available on PATH"
        ))),
        _ => Err(Error::msg(format!(
            "none of {} is available on PATH",
            names.join(", ")
        ))),
    }
}

pub fn run_captured(
    name: &str,
    args: &[&str],
    cwd: Option<&Path>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let output = output(name, args, cwd)?;
    stdout.write_all(&output.stdout)?;
    stderr.write_all(&output.stderr)?;
    status_ok(name, args, &output)
}

pub fn run_merged(
    name: &str,
    args: &[&str],
    cwd: Option<&Path>,
    writer: &mut dyn Write,
) -> Result<(), Error> {
    let output = output(name, args, cwd)?;
    writer.write_all(&output.stdout)?;
    writer.write_all(&output.stderr)?;
    status_ok(name, args, &output)
}

pub fn combined_output(name: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, Error> {
    let output = output(name, args, cwd)?;
    let combined = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    let text = String::from_utf8_lossy(&combined).trim().to_string();
    if output.status.success() {
        Ok(text)
    } else {
        Err(Error::msg(format!(
            "{name} {}: {}\n{text}",
            args.join(" "),
            output.status
        )))
    }
}

/// Run `name` without a terminal and return its stdout. Stderr is captured
/// and only shown in the error.
pub fn captured_stdout(name: &str, args: &[&str]) -> Result<String, Error> {
    let output = output(name, args, None)?;
    status_ok(name, args, &output)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn run_discard(name: &str, args: &[&str]) -> Result<(), Error> {
    let mut cmd = Command::new(name);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let status = cmd.status().map_err(|err| command_error(name, args, err))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::msg(format!("{name} {}: {status}", args.join(" "))))
    }
}

pub fn run_on_stderr(name: &str, args: &[&str]) -> Result<(), Error> {
    run_on_stderr_in(name, args, None, &[])
}

/// Like [`run_on_stderr`], in `cwd` and with extra environment variables.
pub fn run_on_stderr_in(
    name: &str,
    args: &[&str],
    cwd: Option<&Path>,
    envs: &[(&str, &OsStr)],
) -> Result<(), Error> {
    let (stdout, stderr) = stderr_pair()?;
    let mut cmd = Command::new(name);
    cmd.args(args)
        .envs(envs.iter().copied())
        .stdin(Stdio::inherit())
        .stdout(stdout)
        .stderr(stderr);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let status = cmd.status().map_err(|err| command_error(name, args, err))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::msg(format!("{name} {}: {status}", args.join(" "))))
    }
}

/// Run `name` in `cwd` with extra environment variables and return its
/// stdout. Its stderr streams to ours, so progress stays visible.
pub fn stdout_of_in(
    name: &str,
    args: &[&str],
    cwd: Option<&Path>,
    envs: &[(&str, &OsStr)],
) -> Result<String, Error> {
    let (_, stderr) = stderr_pair()?;
    let mut cmd = Command::new(name);
    cmd.args(args)
        .envs(envs.iter().copied())
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(stderr);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let output = cmd.output().map_err(|err| command_error(name, args, err))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(Error::msg(format!(
            "{name} {}: {}",
            args.join(" "),
            output.status
        )))
    }
}

fn output(name: &str, args: &[&str], cwd: Option<&Path>) -> Result<Output, Error> {
    let mut cmd = Command::new(name);
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.output().map_err(|err| command_error(name, args, err))
}

fn status_ok(name: &str, args: &[&str], output: &Output) -> Result<(), Error> {
    if output.status.success() {
        Ok(())
    } else {
        let combined = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
        let text = String::from_utf8_lossy(&combined).trim().to_string();
        Err(Error::msg(format!(
            "{name} {}: {}\n{text}",
            args.join(" "),
            output.status
        )))
    }
}

fn command_error(name: &str, args: &[&str], err: io::Error) -> Error {
    Error::msg(format!("{name} {}: {err}", args.join(" ")))
}

fn stderr_pair() -> Result<(Stdio, Stdio), Error> {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        let fd = io::stderr().as_fd().try_clone_to_owned()?;
        let fd2 = fd.try_clone()?;
        Ok((Stdio::from(fd), Stdio::from(fd2)))
    }

    #[cfg(not(unix))]
    {
        let _ = io::stderr();
        Ok((Stdio::inherit(), Stdio::inherit()))
    }
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        path.is_file()
    }
}
