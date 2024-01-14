mod args;
mod print;

use print::Printer;

use std::{
    env::set_current_dir,
    io::{stdout, IsTerminal, Write},
    os::unix::ffi::OsStrExt,
};

use ansi_term::{Color, Style};
use anyhow::{bail, Context, Error, Result};
use clap::Parser;
use makepkg::{config::Config, Makepkg};
use makepkg::{
    pkgbuild::{OptionState, Pkgbuild},
    Options,
};
use nix::unistd::Uid;

pub fn print_error(style: Style, err: Error) {
    eprint!("{}", style.paint("error"));

    for link in err.chain() {
        eprint!(": {}", link);
    }
    eprintln!();
}

pub fn main() {
    match run() {
        Ok(_) => (),
        Err(e) => {
            print_error(Style::new().fg(Color::Red).bold(), e);
            std::process::exit(1);
        }
    }
}

fn run() -> Result<()> {
    let cli = args::Args::parse();

    if Uid::current().is_root() {
        bail!("running {} as root is not allowed", env!("CARGO_PKG_NAME"))
    }

    if let Some(path) = &cli.chdir {
        set_current_dir(path).with_context(|| format!("failed to cd into {}", path.display()))?;
    }

    let config = if let Some(config) = cli.config {
        Config::from_path(config)?
    } else {
        Config::new()?
    };

    let color = config.build_env("color").enabled() && !cli.nocolor && stdout().is_terminal();
    let makepkg = Makepkg::from_config(config).callbacks(Printer::new(color));
    let mut pkgbuild = Pkgbuild::new(".")?;

    let check = match (cli.check, cli.nocheck) {
        (true, _) => OptionState::Enabled,
        (_, true) => OptionState::Disabled,
        (_, _) => OptionState::Unset,
    };

    let sign = match (cli.sign, cli.sign) {
        (true, _) => OptionState::Enabled,
        (_, true) => OptionState::Disabled,
        (_, _) => OptionState::Unset,
    };

    let options = Options {
        log: cli.log,
        check,
        sign,
        skip_pgp_check: cli.skippgpcheck,
        skip_checksums: cli.skipchecksums,
        no_prepare: cli.noprepare,
        reproducible: std::env::var("SOURCE_DATE_EPOCH").is_ok(),
        ignore_arch: cli.ignorearch,
        clean_build: cli.cleanbuild,
        no_extract: cli.noextract,
        verify_source: cli.verifysource,
        repackage: cli.repackage,
        no_build: cli.nobuild,
        rebuild: cli.force,
        no_archive: cli.noarchive,
        hold_ver: cli.holdver,
    };

    if cli.geninteg {
        let integ = makepkg.geninteg(&options, &pkgbuild)?;
        println!("{}", integ);
        return Ok(());
    }
    if cli.printsrcinfo {
        pkgbuild.write_srcinfo(&mut stdout().lock())?;
        return Ok(());
    }
    if cli.packagelist {
        let mut stdout = stdout().lock();
        for path in makepkg.config().package_list(&pkgbuild)? {
            stdout.write_all(path.as_os_str().as_bytes())?;
        }
        return Ok(());
    }
    if cli.source || cli.allsource {
        makepkg.create_source_package(&options, &pkgbuild, cli.allsource)?;
        return Ok(());
    }

    makepkg.build(&options, &mut pkgbuild)?;
    Ok(())
}
