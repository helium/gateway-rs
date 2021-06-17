use crate::*;
use futures::FutureExt;
use std::ffi::OsStr;
use tokio::process;

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
            Ok(output) => f(&output.stdout),
            Err(err) => Err(Error::from(err)),
        })
        .boxed()
}
