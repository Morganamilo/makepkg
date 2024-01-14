use std::{
    collections::HashMap,
    io::{self, stdout, Write},
    mem::replace,
};

use ansi_term::{Color::*, Style};
use indicatif::{MultiProgress, ProgressBar, ProgressFinish, ProgressStyle};
use makepkg::{
    pkgbuild::Pkgbuild, Callbacks, CommandKind, CommandOutput, DownloadEvent, Event, LogLevel,
    LogMessage,
};

#[derive(Debug, Default, Copy, Clone)]
pub struct Colors {
    pub enabled: bool,
    pub general: Style,
    pub error: Style,
    pub warning: Style,
    pub bold: Style,
    pub action: Style,
}

impl Colors {
    pub fn new() -> Colors {
        Colors {
            enabled: true,
            general: Style::new(),
            error: Style::new().fg(Red).bold(),
            warning: Style::new().fg(Yellow).bold(),
            bold: Style::new().bold(),
            action: Style::new().fg(Blue).bold(),
        }
    }
}

#[derive(Debug)]
pub struct Printer {
    colors: Colors,
    start_line: bool,
    progress: indicatif::MultiProgress,
    bars: HashMap<usize, indicatif::ProgressBar>,
    //term_width: Option<u16>,
    msg_width: u16,
}

impl Callbacks for Printer {
    fn event(&mut self, event: Event) -> io::Result<()> {
        let c = self.colors;

        match event {
            Event::FoundSource(_)
            | Event::Downloading(_)
            | Event::NoExtact(_)
            | Event::Extacting(_)
            | Event::RemovingSrcdir
            | Event::RemovingPkgdir
            | Event::AddingFileToPackage(_)
            | Event::GeneratingPackageFile(_)
            | Event::DownloadingVCS(_, _)
            | Event::ExtractingVCS(_, _)
            | Event::UpdatingVCS(_, _) => {
                writeln!(stdout(), "    {}", c.general.paint(event.to_string()))
            }
            Event::VerifyingChecksum(_) | Event::VerifyingSignature(_) => {
                write!(stdout(), "    {} ...", c.general.paint(event.to_string()))?;
                stdout().flush()
            }
            Event::ChecksumFailed(_, _) | Event::SignatureCheckFailed(_) => {
                writeln!(stdout(), " {}", event)
            }
            Event::ChecksumSkipped(_) | Event::ChecksumPass(_) | Event::SignatureCheckPass(_) => {
                writeln!(stdout(), " {}", c.general.paint(event.to_string()))
            }
            Event::DownloadingCurl(_) => Ok(()),
            _ => {
                writeln!(
                    stdout(),
                    "{} {}",
                    c.action.paint("::"),
                    c.bold.paint(event.to_string())
                )
            }
        }
    }

    fn log(&mut self, level: LogLevel, msg: LogMessage) -> io::Result<()> {
        let c = self.colors;
        match level {
            LogLevel::Warning => {
                writeln!(stdout(), "{}: {}", c.warning.paint(level.to_string()), msg)
            }
            LogLevel::Error => writeln!(stdout(), "{}: {}", c.error.paint(level.to_string()), msg),
            _ => Ok(()),
        }
    }

    fn command_new(
        &mut self,
        _id: usize,
        kind: makepkg::CommandKind,
    ) -> io::Result<makepkg::CommandOutput> {
        self.start_line = true;
        match kind {
            CommandKind::PkgbuildFunction(_) => Ok(CommandOutput::Inherit),
            _ => Ok(CommandOutput::Callback),
        }
    }

    fn command_output(
        &mut self,
        _id: usize,
        _kind: makepkg::CommandKind,
        output: &[u8],
    ) -> io::Result<()> {
        for line in output.split_inclusive(|c| *c == b'\n') {
            {
                if self.start_line {
                    write!(stdout(), "    ")?;
                }
                stdout().write_all(line).unwrap();
                if line.ends_with(&[b'\n']) {
                    self.start_line = true;
                }
            }
        }
        Ok(())
    }

    fn download(&mut self, _pkgbuild: &Pkgbuild, event: DownloadEvent) -> io::Result<()> {
        if let DownloadEvent::Init(download) = event {
            let bar = Self::progress_bar();
            bar.set_message(download.source.file_name().to_string());
            self.bars.insert(download.n, bar);
        } else if let DownloadEvent::Progress(download, dlnow, dltotal) = event {
            let n = download.n;
            let bar = self.bars.get_mut(&n).unwrap();

            if dltotal > 0.0 && bar.length().is_none() {
                let template = format!(
                " {{msg:<{}}} {{bytes:>11}} {{binary_bytes_per_sec:>13}} {{eta_precise}} [{{wide_bar}}] {{percent:>3}}%",
                self.msg_width,
            );

                bar.set_length(dltotal as _);
                bar.set_style(
                    ProgressStyle::default_bar()
                        .template(&template)
                        .unwrap()
                        .progress_chars("##-"),
                );
                let bar2 = replace(bar, ProgressBar::hidden());
                *bar = self.progress.add(bar2);
            }
            bar.set_position(dlnow as _);
        } else if let DownloadEvent::DownloadEnd = event {
            self.bars.clear();
            println!();
        }
        Ok(())
    }
}

impl Printer {
    pub fn new(color: bool) -> Self {
        let colors = if color {
            Colors::new()
        } else {
            Colors::default()
        };

        let term_width = terminal_size::terminal_size().map(|s| s.0 .0);
        let msg_width = term_width.unwrap_or(50) * 6 / 10 - 36;

        Printer {
            colors,
            start_line: true,
            //term_width,
            msg_width,
            progress: MultiProgress::new(),
            bars: HashMap::new(),
        }
    }

    fn progress_bar() -> ProgressBar {
        let template = " {msg}";

        let style = ProgressStyle::with_template(&template).unwrap();

        ProgressBar::hidden()
            .with_style(style)
            .with_finish(ProgressFinish::Abandon)
    }
}
