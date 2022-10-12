use crate::{Future, Result};
use futures::FutureExt;
use std::ffi::OsStr;
use tokio::process;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error {0:?}")]
    Io(#[from] std::io::Error),
    #[error("command error")]
    Exit(std::process::ExitStatus),
    #[error("terminated with signal {0:?}")]
    Signal(Option<i32>),
}

pub fn get<U, I, S, R, F>(url: U, args: I, f: F) -> Future<R>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    U: AsRef<OsStr>,
    F: FnOnce(&[u8]) -> Result<R> + std::marker::Send + 'static,
{
    process::Command::new("curl")
        .kill_on_drop(true)
        .args(args)
        .arg("-f")
        .arg(&url)
        .output()
        .map(move |result| match result {
            Ok(output) if output.status.success() => f(&output.stdout),
            Ok(output) if output.status.code().is_none() => {
                cfg_if::cfg_if! {
                    if #[cfg(any(unix, macos))] {
                        use std::os::unix::process::ExitStatusExt;
                        Err(Error::Signal(output.status.signal()).into())
                    } else {
                        Err(Error::Signal(None).into())
                    }
                }
            }
            Ok(output) => Err(Error::Exit(output.status).into()),
            Err(err) => Err(Error::Io(err).into()),
        })
        .boxed()
}
