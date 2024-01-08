use std::process::Command;

use crate::error::{CommandErrorExt, Context, Result};

/*
pub fn deptest<'a, I: Iterator<Item = &'a str>>(pkgs: I) -> Result<Vec<String>> {
    read_pacman(&["-T"], pkgs)
}

pub fn installed() -> Result<Vec<String>> {
    let pkgs = read_pacman(&["-Qq"], None.into_iter())?;
    Ok(pkgs)
}
*/

pub fn buildinfo_installed() -> Result<Vec<String>> {
    let mut installed = Vec::new();
    let mut current = String::new();
    let pkgs = read_pacman(&["-Qi"], None.into_iter())?;

    for pkg in pkgs {
        if pkg.starts_with("Name") {
            current.push_str(pkg.split(": ").nth(1).unwrap().trim());
        }
        if pkg.starts_with("Version") {
            current.push('-');
            current.push_str(pkg.split(": ").nth(1).unwrap().trim());
        }
        if pkg.starts_with("Architecture") {
            current.push('-');
            current.push_str(pkg.split(": ").nth(1).unwrap().trim());
        }

        if pkg.is_empty() {
            installed.push(current);
            current = String::new();
        }
    }

    installed.push(current);

    Ok(installed)
}

fn read_pacman<'a, S, I>(args: &[S], pkgs: I) -> Result<Vec<String>>
where
    S: AsRef<str>,
    I: Iterator<Item = &'a str>,
{
    let mut command = Command::new("pacman");
    for arg in args {
        command.arg(arg.as_ref());
    }
    command.arg("--");

    for pkg in pkgs {
        command.arg(pkg);
    }

    let output = command
        .output()
        .cmd_context(&command, Context::QueryPacman)?;

    let output = String::from_utf8(output.stdout).cmd_context(&command, Context::QueryPacman)?;
    Ok(output.trim().lines().map(|l| l.to_string()).collect())
}

/*
pub fn run_pacman<'a, I: Iterator<Item = &'a str>>(op: &str, args: &[&str], pkgs: I) -> Result<()> {
    let mut command = Command::new("sudo");
    command.arg("pacman").arg(op).args(args).arg("--");
    command.args(pkgs);

    command.status().cmd_context(&command, Context::RunPacman)?;
    Ok(())
}
*/
