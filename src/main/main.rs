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
use makepkg::{pkgbuild::Pkgbuild, Options};
use nix::unistd::Uid;

pub fn print_error(style: Style, err: Error) {
    eprint!("{}", style.paint("error"));

    for link in err.chain() {
        let merr = err.downcast_ref::<makepkg::error::Error>();
        eprint!(": {}", link);
        if let Some(makepkg::error::Error::AlreadyBuilt(_)) = merr {
            eprint!(" (use -f to overwrite)");
        }
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

    let mut options = Options {
        no_deps: cli.nodeps,
        sync_deps: cli.syncdeps,
        install: cli.install,
        log: cli.log,
        clean: false,
        clean_build: cli.cleanbuild,
        ignore_arch: cli.ignorearch,
        hold_ver: cli.holdver,
        no_download: false,
        no_checksums: cli.skipchecksums || cli.skipinteg,
        no_signatures: cli.skippgpcheck || cli.skipinteg,
        no_verify: cli.noverify,
        no_extract: cli.noextract,
        no_prepare: cli.noprepare,
        no_build: cli.nobuild,
        keep_pkg: false,
        no_check: cli.nocheck,
        no_package: false,
        no_archive: cli.noarchive,
        rebuild: cli.force,
    };

    if cli.repackage {
        options.repackage();
    } else if cli.verifysource {
        options.verify_source();
    } else if cli.nobuild {
        options.no_build();
    }

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
