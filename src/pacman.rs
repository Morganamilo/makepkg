use std::process::Command;

use crate::{
    callback::CommandKind,
    error::{CommandOutputExt, Context, Result},
    pkgbuild::Pkgbuild,
    run::CommandOutput,
    Makepkg,
};

/*
pub fn deptest<'a, I: Iterator<Item = &'a str>>(pkgs: I) -> Result<Vec<String>> {
    read_pacman(&["-T"], pkgs)
}

pub fn installed() -> Result<Vec<String>> {
    let pkgs = read_pacman(&["-Qq"], None.into_iter())?;
    Ok(pkgs)
}
*/

pub fn buildinfo_installed(makepkg: &Makepkg, pkgbuild: &Pkgbuild) -> Result<Vec<String>> {
    let mut installed = Vec::new();
    let mut current = String::new();
    let pkgs = read_pacman(makepkg, pkgbuild, &["-Qi"], None.into_iter())?;

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

    if !current.is_empty() {
        installed.push(current);
    }

    Ok(installed)
}

fn read_pacman<'a, S, I>(
    makepkg: &Makepkg,
    pkgbuild: &Pkgbuild,
    args: &[S],
    pkgs: I,
) -> Result<Vec<String>>
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
        .process_read(makepkg, CommandKind::BuildingPackage(pkgbuild))
        .read(&command, Context::QueryPacman)?;

    Ok(output.lines().map(|l| l.to_string()).collect())
}

/*
pub fn run_pacman<'a, I: Iterator<Item = &'a str>>(op: &str, args: &[&str], pkgs: I) -> Result<()> {
    let mut command = self.command("sudo");
    command.arg("pacman").arg(op).args(args).arg("--");
    command.args(pkgs);

    command.st//atus().cmd_context(&command, Context::RunPacman)?;
    Ok(())
}
*/
