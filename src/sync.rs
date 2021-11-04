use crate::{Error, Result};
use slog::{warn, Logger};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
pub struct MessageSender<T>(pub(crate) mpsc::Sender<T>);
pub struct MessageReceiver<T>(mpsc::Receiver<T>);

pub fn message_channel<T>(size: usize) -> (MessageSender<T>, MessageReceiver<T>) {
    let (tx, rx) = mpsc::channel(size);
    (MessageSender(tx), MessageReceiver(rx))
}

impl<T> MessageReceiver<T> {
    pub async fn recv(&mut self) -> Option<T> {
        self.0.recv().await
    }
}

impl<T> Clone for MessageSender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[derive(Debug)]
pub struct ResponseSender<T>(oneshot::Sender<T>);
pub struct ResponseReceiver<T>(oneshot::Receiver<T>);

pub fn response_channel<T>() -> (ResponseSender<T>, ResponseReceiver<T>) {
    let (tx, rx) = oneshot::channel();
    (ResponseSender(tx), ResponseReceiver(rx))
}

impl<T: std::fmt::Debug> ResponseSender<T> {
    pub fn send_response(self, msg: T, logger: &Logger) {
        match self.0.send(msg) {
            Ok(()) => (),
            Err(err) => warn!(logger, "ignoring channel error: {:?}", err),
        }
    }
}

impl<T> ResponseReceiver<T> {
    pub async fn recv(self) -> Result<T> {
        self.0.await.map_err(|_| Error::channel())
    }
}
