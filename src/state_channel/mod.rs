mod channel;
mod message;

pub use channel::{check_active, StateChannel, StateChannelCausality, StateChannelValidation};
pub use message::StateChannelMessage;
