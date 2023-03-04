use crate::{Error, Result};
use futures::TryFutureExt;
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

#[derive(Debug)]
pub struct MessageSender<T>(pub(crate) mpsc::Sender<T>);
#[derive(Debug)]
pub struct MessageReceiver<T>(mpsc::Receiver<T>);
pub struct MessageChannel<T> {
    pub(crate) tx: MessageSender<T>,
    pub(crate) rx: MessageReceiver<T>,
}

impl<T> From<(mpsc::Sender<T>, mpsc::Receiver<T>)> for MessageChannel<T> {
    fn from(value: (mpsc::Sender<T>, mpsc::Receiver<T>)) -> Self {
        Self {
            tx: MessageSender(value.0),
            rx: MessageReceiver(value.1),
        }
    }
}

impl<T> MessageChannel<T> {
    pub fn new(size: usize) -> Self {
        mpsc::channel(size).into()
    }

    pub async fn recv(&mut self) -> Option<T> {
        self.rx.recv().await
    }

    pub fn sender(&self) -> MessageSender<T> {
        self.tx.clone()
    }
}

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

impl<T> MessageSender<T> {
    pub async fn send(&self, msg: T) {
        _ = self.0.send(msg).await
    }

    pub async fn request<R, F>(&self, req: F) -> Result<R>
    where
        F: FnOnce(ResponseSender<R>) -> T,
    {
        let (tx, rx) = response_channel();
        self.0.send(req(tx)).map_err(|_| Error::channel()).await?;
        rx.recv().await
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
    pub fn send(self, msg: T) {
        match self.0.send(msg) {
            Ok(()) => (),
            Err(err) => warn!(?err, "ignoring channel error"),
        }
    }
}

impl<T> ResponseReceiver<T> {
    pub async fn recv(self) -> Result<T> {
        self.0.map_err(|_| Error::channel()).await
    }
}
