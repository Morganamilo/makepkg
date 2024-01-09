use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    mem::replace,
    path::PathBuf,
    result::Result as StdResult,
    time::Duration,
};

use curl::{
    easy::{Easy2, Handler, WriteError},
    multi::{Easy2Handle, Multi},
};

use crate::{
    callback::Event,
    config::PkgbuildDirs,
    error::{Context, DownloadError, IOContext, IOErrorExt, Result},
    fs::{open, rename},
    pkgbuild::Source,
    Makepkg,
};

pub struct Handle<'a> {
    makepkg: &'a Makepkg,
    source: Source,
    file: File,
    temp_path: PathBuf,
    final_path: PathBuf,
    err: Result<()>,
}

impl<'a> Handler for Handle<'a> {
    fn write(&mut self, data: &[u8]) -> StdResult<usize, WriteError> {
        let err = self.file.write_all(data).context(
            Context::RetrieveSources,
            IOContext::WriteDownload(self.source.file_name().to_string()),
        );
        if let Err(err) = err {
            self.err = Err(err.into());
            Err(WriteError::Pause)
        } else {
            Ok(data.len())
        }
    }

    fn progress(&mut self, dltotal: f64, dlnow: f64, _ultotal: f64, _ulnow: f64) -> bool {
        self.makepkg.progress(self.source.clone(), dltotal, dlnow);
        true
    }

    fn seek(&mut self, seek: SeekFrom) -> curl::easy::SeekResult {
        let err = self.file.seek(seek).context(
            Context::RetrieveSources,
            IOContext::Seek(self.temp_path.clone()),
        );
        if let Err(err) = err {
            self.err = Err(err.into());
            curl::easy::SeekResult::Fail
        } else {
            curl::easy::SeekResult::Ok
        }
    }
}

impl Makepkg {
    pub(crate) fn download_curl_sources(
        &self,
        dirs: &PkgbuildDirs,
        mut sources: Vec<&Source>,
    ) -> Result<()> {
        let curlm = Multi::new();
        let max_downloads = 8;
        let mut handles = Vec::new();
        let mut running = 0;

        while running > 0 || !sources.is_empty() {
            while running < max_downloads && !sources.is_empty() {
                if let Some(source) = sources.pop() {
                    let curl = self.make_payload(dirs, source)?;
                    self.event(Event::Downloading(source.file_name().to_string()));
                    let handle = curlm.add2(curl)?;
                    handles.push(handle);
                    running += 1;
                }
            }

            running = curlm.perform()?;
            curlm.wait(&mut [], Duration::from_secs(1))?;

            handle_messages(&curlm, &mut handles);

            if let Some(handler) = handles.iter_mut().find(|h| h.get_ref().err.is_err()) {
                let err = replace(&mut handler.get_mut().err, Ok(()));
                return err;
            }
        }

        drop(handles);
        Ok(())
    }

    fn make_payload(&self, dirs: &PkgbuildDirs, source: &Source) -> Result<Easy2<Handle>> {
        let name = source.file_name();
        let final_path = dirs.srcdest.join(name);
        let mut temp_path = final_path.clone();
        if let Some(extension) = temp_path.extension() {
            let mut extension = extension.to_os_string();
            extension.push(".part");
            temp_path.set_extension(extension);
        } else {
            temp_path.set_extension("part");
        }
        let mut file = File::options();
        file.create(true).write(true);
        let mut file = open(&file, &temp_path, Context::RetrieveSources)?;
        let len = file
            .seek(SeekFrom::End(0))
            .context(Context::RetrieveSources, IOContext::Seek(temp_path.clone()))?;
        let mut curl = Easy2::new(Handle {
            makepkg: self,
            file,
            temp_path,
            final_path,
            source: source.clone(),
            err: Ok(()),
        });
        curl_set_ops(&mut curl, source)?;
        curl.resume_from(len)?;
        Ok(curl)
    }
}

fn handle_messages(curlm: &Multi, handles: &mut [Easy2Handle<Handle>]) {
    curlm.messages(|m| {
        for handle in handles.iter_mut() {
            if let Some(res) = m.result_for2(handle) {
                let response = handle.response_code().unwrap_or(0);
                let context = handle.get_mut();

                if let Err(e) = res {
                    context.err = Err(e.into());
                    return;
                }
                if !(200..300).contains(&response) {
                    context.err =
                        Err(DownloadError::Status(context.source.clone(), response).into());
                    return;
                }

                if let Err(err) = rename(
                    &context.temp_path,
                    &context.final_path,
                    Context::RetrieveSources,
                ) {
                    context.err = Err(err);
                    return;
                }
            };
        }
    });
}

fn curl_set_ops<T>(curl: &mut Easy2<T>, source: &Source) -> Result<()> {
    curl.useragent(&format!(
        "{}/{}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    ))?;
    curl.max_redirections(10)?;
    curl.follow_location(true)?;
    curl.connect_timeout(Duration::from_secs(10))?;
    curl.progress(true)?;
    curl.tcp_keepidle(Duration::from_secs(1))?;
    curl.url(&source.url)?;
    curl.get(true)?;
    Ok(())
}
