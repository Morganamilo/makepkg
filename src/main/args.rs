use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug, Default)]
#[command(author, version, about)]
pub struct Args {
    #[arg(long, short = 'D')]
    pub chdir: Option<PathBuf>,
    #[arg(long, short = 'm')]
    pub nocolor: bool,
    #[arg(long, short = 'L')]
    pub log: bool,
    #[arg(long, short)]
    pub force: bool,
    #[arg(long)]
    pub packagelist: bool,
    #[arg(long)]
    pub printsrcinfo: bool,
    #[arg(long, short = 'g')]
    pub geninteg: bool,
    #[arg(long, short = 'd')]
    pub nodeps: bool,
    #[arg(long)]
    pub skipinteg: bool,
    #[arg(long)]
    pub skipchecksums: bool,
    #[arg(long)]
    pub skippgpcheck: bool,
    #[clap(long, overrides_with = "check")]
    pub nocheck: bool,
    #[clap(long)]
    pub noverify: bool,
    #[clap(long, overrides_with = "nocheck")]
    pub check: bool,
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long, short = 'A')]
    pub ignorearch: bool,
    #[arg(long, short = 'e')]
    pub noextract: bool,
    #[arg(long)]
    pub verifysource: bool,
    #[arg(long, short = 'C')]
    pub cleanbuild: bool,
    #[arg(long, short)]
    pub clean: bool,
    #[arg(long)]
    pub noprepare: bool,
    #[arg(long, short = 'o')]
    pub nobuild: bool,
    #[arg(long, short = 'R')]
    pub repackage: bool,
    #[arg(long)]
    pub noarchive: bool,
    #[clap(long, overrides_with = "nosign")]
    pub sign: bool,
    #[clap(long, overrides_with = "sign")]
    pub nosign: bool,
    #[arg(long, short = 'S')]
    pub source: bool,
    #[arg(long)]
    pub allsource: bool,
    #[arg(long)]
    pub holdver: bool,

    #[arg(long, short)]
    pub rmdeps: bool,
    #[arg(long, short)]
    pub syncdeps: bool,
    #[arg(long, short)]
    pub install: bool,
    #[arg(long)]
    pub asdeps: bool,
    #[arg(long)]
    pub needed: bool,
    #[arg(long)]
    pub noconfirm: bool,
    #[arg(long)]
    pub noprogressbar: bool,
}
